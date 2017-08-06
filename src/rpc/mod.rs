pub mod proto;
mod reader;
mod writer;
mod errors;
mod client;
mod processor;

use std::{io, str};
use std::net::{TcpListener, TcpStream, Ipv4Addr, SocketAddrV4};
use std::collections::HashMap;

use slog::Logger;
use serde_json;
use amy;

pub use self::proto::resource;
pub use self::errors::{Result, ResultExt, ErrorKind, Error};
use self::proto::ws;
use self::client::{Incoming, Client};
use self::processor::Processor;
use handle;
use torrent;
use CONFIG;

#[derive(Debug)]
pub enum CtlMessage {
    Extant(resource::Resource),
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
                    _ => self.handle_conn(not),
                }
            }
        }
        loop {}
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
                Ok(true) => {
                    debug!(self.l, "Succesfully upgraded conn");
                    self.clients.insert(id, i.into());
                }
                Ok(false) => {
                    self.incoming.insert(id, i);
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
