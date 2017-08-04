pub mod proto;
mod reader;
mod writer;
mod errors;

pub use self::proto::{Request, Response, TorrentInfo};
pub use self::errors::{Result, ErrorKind};
use self::reader::Reader;
use self::writer::Writer;

use std::{io, time, result, str};
use io::Write;
use std::net::{TcpListener, TcpStream, Ipv4Addr, SocketAddrV4};
use slog::Logger;
use {amy, base64, httparse, handle, CONFIG};
use ring::digest;
use util::{aread, IOR};
// TODO: Allow customizing this
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
    clients: HashMap<usize, Client>,
    incoming: HashMap<usize, Incoming>,
    l: Logger,
}

struct Client {
    r: Reader,
    w: Writer,
    conn: TcpStream,
    last_action: time::Instant,
}

struct Incoming {
    key: Option<String>,
    conn: TcpStream,
    buf: [u8; 1024],
    pos: usize,
    last_action: time::Instant,
}

macro_rules! id_match {
    ($req:expr, $resp:expr, $s:expr, $body:expr) => (
        {
            lazy_static! {
                static ref M: (String, String, usize) = {
                    let mut s = $s.to_owned();
                    let idx = s.find("{}").unwrap();
                    let mut remaining = s.split_off(idx);
                    let end = remaining.split_off(2);
                    (s, end, idx)
                };
            };
            let start = &M.0;
            let end = &M.1;
            let idx = M.2;
            if $req.url().starts_with(start) && $req.url().ends_with(end) {
                let len = $req.url().len();
                let val = &$req.url()[idx..(len - end.len())];
                if let Ok(i) = val.parse::<usize>() {
                    $resp = Ok($body(i));
                } else {
                    $resp = Err(format!("{} is not a valid integer!", val));
                }
            }
        }
    );
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
                match c.r.read(&mut c.conn) {
                    Ok(None) => { }
                    Ok(Some(m)) => {
                        debug!(self.l, "Got a message from the client: {:?}", m);
                    }
                    Err(_) => return,
                }
            }
            if not.event.readable() {
                if c.w.write(&mut c.conn).is_err() {
                    return;
                }
            }
            self.clients.insert(not.id, c);
        }
    }
}

impl Incoming {
    pub fn new(conn: TcpStream) -> Incoming {
        conn.set_nonblocking(true).unwrap();
        Incoming {
            conn,
            buf: [0; 1024],
            pos: 0,
            last_action: time::Instant::now(),
            key: None,
        }
    }

    /// Result indicates if the Incoming connection is
    /// valid to be upgraded into a Client
    pub fn readable(&mut self) -> io::Result<bool> {
        loop {
            match aread(&mut self.buf[self.pos..], &mut self.conn) {
                // TODO: Consider more
                IOR::Complete => return Err(io::ErrorKind::InvalidData.into()),
                IOR::Incomplete(a) => {
                    self.pos += a;
                    let mut headers = [httparse::EMPTY_HEADER; 24];
                    let mut req = httparse::Request::new(&mut headers);
                    match req.parse(&self.buf[..self.pos]) {
                        Ok(httparse::Status::Partial) => continue,
                        Ok(httparse::Status::Complete(_)) => {
                            if let Ok(k) = validate_upgrade(req) {
                                self.key = Some(k);
                                return Ok(true);
                            } else {
                                return Err(io::ErrorKind::InvalidData.into());
                            }
                        }
                        Err(_) => return Err(io::ErrorKind::InvalidData.into()),
                    }
                }
                IOR::Blocked => return Ok(false),
                IOR::EOF => return Err(io::ErrorKind::UnexpectedEof.into()),
                IOR::Err(e) => return Err(e),
            }
        }
    }
}

impl Into<Client> for Incoming {
    fn into(mut self) -> Client {
        let mut ctx = digest::Context::new(&digest::SHA1);
        let magic = self.key.unwrap() + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
        ctx.update(magic.as_bytes());
        let digest = ctx.finish();
        let accept = base64::encode(digest.as_ref());
        let lines = vec![
            format!("HTTP/1.1 101 Switching Protocols"),
            format!("Connection: upgrade"),
            format!("Upgrade: websocket"),
            format!("Sec-WebSocket-Accept: {}", accept),
        ];
        let data = lines.join("\r\n") + "\r\n\r\n";
        // Ignore error, it'll pop up again anyways
        self.conn.write(data.as_bytes());

        Client {
            r: Reader::new(),
            w: Writer::new(),
            conn: self.conn,
            last_action: time::Instant::now(),
        }
    }
}

fn validate_upgrade(req: httparse::Request) -> result::Result<String, ()> {
    if !req.method.map(|m| m == "GET").unwrap_or(false) {
        return Err(());
    }

    let mut conn = None;
    let mut upgrade = None;
    let mut key = None;
    let mut version = None;

    for header in req.headers.iter() {
        if header.name == "Connection" {
            conn = str::from_utf8(header.value).ok();
        }
        if header.name == "Upgrade" {
            upgrade = str::from_utf8(header.value).ok();
        }
        if header.name == "Sec-WebSocket-Key" {
            key = str::from_utf8(header.value).ok();
        }
        if header.name == "Sec-WebSocket-Version" {
            version = str::from_utf8(header.value).ok();
        }
    }

    if conn != Some("Upgrade") {
        return Err(());
    }
    if upgrade != Some("websocket") {
        return Err(());
    }

    if version != Some("13") {
        return Err(());
    }

    if let Some(k) = key {
        return Ok(k.to_owned());
    }
    return Err(());
}
