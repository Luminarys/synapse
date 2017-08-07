pub mod proto;
mod reader;
mod writer;
mod errors;
mod client;
mod processor;
mod transfer;

use std::{io, str};
use std::net::{TcpListener, TcpStream, Ipv4Addr, SocketAddrV4};
use std::collections::HashMap;

use slog::Logger;
use serde_json;
use amy;

pub use self::proto::resource;
use self::proto::message::SMessage;
pub use self::errors::{Result, ResultExt, ErrorKind, Error};
use self::proto::ws;
use self::client::{Incoming, IncomingStatus, Client};
use self::processor::{Processor, TransferKind};
use self::transfer::{Transfers, TransferResult};
use bencode;
use handle;
use torrent;
use CONFIG;

#[derive(Debug)]
pub enum CtlMessage {
    Extant(Vec<resource::Resource>),
    Update(Vec<resource::SResourceUpdate<'static>>),
    Removed(Vec<u64>),
    Shutdown,
}

#[derive(Debug)]
pub enum Message {
    UpdateTorrent(resource::CResourceUpdate),
    UpdateFile {
        id: u64,
        torrent_id: u64,
        priority: u8,
    },
    RemoveTorrent(u64),
    RemovePeer { id: u64, torrent_id: u64 },
    RemoveTracker { id: u64, torrent_id: u64 },
    Torrent(torrent::Info),
}

#[allow(dead_code)]
pub struct RPC {
    poll: amy::Poller,
    reg: amy::Registrar,
    ch: handle::Handle<CtlMessage, Message>,
    listener: TcpListener,
    lid: usize,
    processor: Processor,
    transfers: Transfers,
    clients: HashMap<usize, Client>,
    incoming: HashMap<usize, Incoming>,
    l: Logger,
}

impl RPC {
    pub fn start(creg: &mut amy::Registrar) -> io::Result<handle::Handle<Message, CtlMessage>> {
        let poll = amy::Poller::new()?;
        let mut reg = poll.get_registrar()?;
        let (ch, dh) = handle::Handle::new(creg, &mut reg)?;

        let ip = Ipv4Addr::new(0, 0, 0, 0);
        let port = CONFIG.rpc_port;
        let listener = TcpListener::bind(SocketAddrV4::new(ip, port))?;
        listener.set_nonblocking(true)?;
        let lid = reg.register(&listener, amy::Event::Both)?;

        dh.run("rpc", move |ch, l| {
            RPC {
                ch,
                poll,
                reg,
                listener,
                lid,
                clients: HashMap::new(),
                incoming: HashMap::new(),
                processor: Processor::new(),
                transfers: Transfers::new(),
                l,
            }.run()
        });
        Ok(ch)
    }

    pub fn run(&mut self) {
        debug!(self.l, "Running RPC!");
        'outer: while let Ok(res) = self.poll.wait(15) {
            for not in res {
                match not.id {
                    id if id == self.lid => self.handle_accept(),
                    id if id == self.ch.rx.get_id() => {
                        if self.handle_ctl() {
                            return;
                        }
                    }
                    id if self.incoming.contains_key(&id) => self.handle_incoming(id),
                    id if self.transfers.contains(id) => self.handle_transfer(not),
                    _ => self.handle_conn(not),
                }
            }
        }
    }

    fn handle_ctl(&mut self) -> bool {
        while let Ok(m) = self.ch.recv() {
            match m {
                CtlMessage::Shutdown => return true,
                m => {
                    let msgs: Vec<_> = {
                        self.processor.handle_ctl(m).into_iter().map(|(c, m)| (c, serde_json::to_string(&m).unwrap())).collect()
                    };
                    for (c, m) in msgs {
                        let res = match self.clients.get_mut(&c) {
                            Some(client) => client.send(ws::Frame::Text(m)),
                            None => { warn!(self.l, "Processor requested a message transfer to a nonexistent client!"); Ok(()) },
                        };
                        if res.is_err() {
                            let client = self.clients.remove(&c).unwrap();
                            self.remove_client(c, client);
                        }
                    }
                }
            }
        }
        false
    }

    fn handle_transfer(&mut self, not: amy::Notification) {
        if not.event.readable() {
            match self.transfers.readable(not.id) {
                TransferResult::Incomplete => { }
                TransferResult::Torrent { conn, data, path } => {
                    debug!(self.l, "Got torrent via HTTP transfer!");
                    self.reg.deregister(&conn);
                    // TODO: Send this to the client in an error msg
                    match bencode::decode_buf(&data) {
                        Ok(b) => {
                            if let Ok(i) = torrent::info::Info::from_bencode(b) {
                                self.ch.send(Message::Torrent(i));
                            } else {
                                warn!(self.l, "Failed to parse torrent!");
                            }
                        }
                        Err(e) => {
                            warn!(self.l, "Failed to decode BE data: {}!", e);
                        }
                    }
                }
                TransferResult::Error { conn, err, client: id } => {
                    self.reg.deregister(&conn);
                    let res = self.clients.get_mut(&id).map(|c| c.send(
                            ws::Frame::Text(serde_json::to_string(&SMessage::TransferFailed(err)).unwrap())
                    )).unwrap_or(Ok(()));
                    if res.is_err() {
                        let client = self.clients.remove(&id).unwrap();
                        self.remove_client(id, client);
                    }
                }
            }
        } else {
        }
    }

    fn handle_accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((conn, ip)) => {
                    debug!(self.l, "Accepted new connection from {:?}!", ip);
                    let id = self.reg.register(&conn, amy::Event::Both).unwrap();
                    self.incoming.insert(id, Incoming::new(conn));
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    error!(self.l, "Failed to accept conn: {}", e);
                }
            }
        }
    }

    fn handle_incoming(&mut self, id: usize) {
        if let Some(mut i) = self.incoming.remove(&id) {
            match i.readable() {
                Ok(IncomingStatus::Upgrade) => {
                    debug!(self.l, "Succesfully upgraded conn");
                    self.clients.insert(id, i.into());
                }
                Ok(IncomingStatus::Incomplete) => {
                    self.incoming.insert(id, i);
                }
                Ok(IncomingStatus::Transfer { data, token }) => {
                    warn!(self.l, "Attempting to initiate transfer");
                    match self.processor.get_transfer(token) {
                        Some((client, serial, TransferKind::UploadTorrent { path, size })) => {
                            self.transfers.add_torrent(id, client, serial, i.into(), data, path, size);
                        }
                        Some(_) => warn!(self.l, "Unimplemented transfer type ignored"),
                        None => {
                            warn!(self.l, "Transfer used invalid token");
                            // TODO: Handle downloads and other uploads
                        }
                    }
                }
                Err(e) => {
                    debug!(self.l, "Incoming ws upgrade failed: {}", e);
                    self.reg.deregister::<TcpStream>(&i.into()).unwrap();
                }
            }
        }
    }

    fn handle_conn(&mut self, not: amy::Notification) {
        if let Some(mut c) = self.clients.remove(&not.id) {
            if not.event.readable() {
                let res = 'outer: loop {
                    match c.read() {
                        Ok(None) => break true,
                        Ok(Some(ws::Frame::Text(data))) => {
                            match serde_json::from_str(&data) {
                                Ok(m) => {
                                    trace!(self.l, "Got a message from the client: {:?}", m);
                                    let (msgs, rm) = self.processor.handle_client(not.id, m);
                                    if let Some(m) = rm {
                                        self.ch.send(m).unwrap();
                                    }
                                    for msg in msgs {
                                        if c.send(
                                            ws::Frame::Text(serde_json::to_string(&msg).unwrap()),
                                            ).is_err()
                                        {
                                            break 'outer false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    info!(
                                        self.l,
                                        "Client sent an invalid message, disconnecting: {}",
                                        e
                                        );
                                    break false;
                                }
                            }
                        }
                        Ok(Some(_)) => break false,
                        Err(_) => break false,
                    }
                };
                if !res {
                    self.remove_client(not.id, c);
                    return;
                }
            }
            if not.event.writable() {
                if c.write().is_err() {
                    self.remove_client(not.id, c);
                    return;
                }
            }
            self.clients.insert(not.id, c);
        }
    }

    fn remove_client(&mut self, id: usize, client: Client) {
        self.processor.remove_client(id);
        self.reg.deregister::<TcpStream>(&client.into()).unwrap();
    }
}
