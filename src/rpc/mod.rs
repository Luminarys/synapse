pub mod proto;
mod reader;
mod writer;
mod errors;
mod client;
mod processor;

pub use self::proto::{Request, Response, TorrentInfo};
pub use self::errors::{Result, ResultExt, ErrorKind, Error};
use self::client::{Incoming, Client};
use self::processor::Processor;

use std::{io, str};
use std::net::{TcpListener, Ipv4Addr, SocketAddrV4};
use slog::Logger;
use {amy, handle, CONFIG};
use std::collections::HashMap;

#[derive(Debug)]
pub enum CMessage {
    Response(Response),
    Shutdown,
}

#[allow(dead_code)]
pub struct RPC {
    poll: amy::Poller,
    reg: amy::Registrar,
    ch: handle::Handle<CMessage, Request>,
    listener: TcpListener,
    lid: usize,
    processor: Processor,
    clients: HashMap<usize, Client>,
    incoming: HashMap<usize, Incoming>,
    l: Logger,
}

impl RPC {
    pub fn start(creg: &mut amy::Registrar) -> io::Result<handle::Handle<Request, CMessage>> {
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
                        if let Ok(CMessage::Shutdown) = self.ch.recv() {
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
                Err(e) => debug!(self.l, "Incoming ws upgrade failed: {}", e),
            }
        }
    }

    fn handle_conn(&mut self, not: amy::Notification) {
        if let Some(mut c) = self.clients.remove(&not.id) {
            if not.event.readable() {
                loop {
                    match c.read() {
                        Ok(None) => break,
                        Ok(Some(m)) => {
                            debug!(self.l, "Got a message from the client: {:?}", m);
                        }
                        Err(_) => return,
                    }
                }
            }
            if not.event.writable() {
                if c.write().is_err() {
                    return;
                }
            }
            self.clients.insert(not.id, c);
        }
    }
}
