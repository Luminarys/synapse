use std::{thread, time};
use std::net::{TcpListener, TcpStream, Ipv4Addr, SocketAddrV4};
use slog::Logger;
use torrent::Status;
use std::io;
use {amy, serde_json, torrent, handle, CONFIG};
// TODO: Allow customizing this
use std::collections::HashMap;

mod reader;
mod writer;
mod errors;
mod proto;

pub use self::proto::{Request, Response, TorrentInfo};
pub use self::errors::{Result, ErrorKind};

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
    clients: HashMap<usize, TcpStream>,
    incoming: HashMap<usize, TcpStream>,
    l: Logger,
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

        dh.run("rpc", move |ch, l| RPC {
            ch,
            poll,
            reg,
            listener,
            lid,
            clients: HashMap::new(),
            incoming: HashMap::new(),
            l
        }.run());
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
        loop { }
    }

    fn handle_accept(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((conn, ip)) => {
                    debug!(self.l, "Accepted new connection from {:?}!", ip);
                    let id = self.reg.register(&conn, amy::Event::Both).unwrap();
                    self.incoming.insert(id, conn);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => { error!(self.l, "Failed to accept conn: {}", e); }
            }
        }
    }

    fn handle_incoming(&mut self, id: usize) {
    }

    fn handle_conn(&mut self, not: amy::Notification) {
    }

    /*
    fn handle_request(&mut self, mut request: tiny_http::Request) -> Result<(), ()> {
        debug!(self.l, "New Req {:?}, {:?}!", request.url(), request.method());
        let mut resp = Err("Invalid URL".to_owned());
        id_match!(request, resp, "/torrent/{}/info", |i| Request::TorrentInfo(i));
        id_match!(request, resp, "/torrent/{}/pause", |i| Request::PauseTorrent(i));
        id_match!(request, resp, "/torrent/{}/resume", |i| Request::ResumeTorrent(i));
        id_match!(request, resp, "/torrent/{}/remove", |i| Request::RemoveTorrent(i));
        id_match!(request, resp, "/throttle/upload/{}", |i| Request::ThrottleUpload(i));
        id_match!(request, resp, "/throttle/download/{}", |i| Request::ThrottleDownload(i));
        if request.url() == "/shutdown" {
            let r = serde_json::to_string(&Response::Ack).unwrap();
            let resp = tiny_http::Response::from_string(r);
            request.respond(resp).unwrap();
            return Err(());
        };
        if request.url() == "/torrent/list" {
            resp = Ok(Request::ListTorrents);
        };
        if request.url() == "/torrent" {
            let mut data = Vec::new();
            request.as_reader().read_to_end(&mut data).unwrap();
            resp = match bencode::decode_buf(&data) {
                Ok(b) => Ok(Request::AddTorrent(b)),
                Err(_) => Err("Bad torrent!".to_owned()),
            };
        }

        let resp = match resp {
            Ok(rpc) => {
                debug!(self.l, "Request validated, sending to ctrl!");
                if let Ok(()) = self.ch.send(rpc) {
                    let resp;
                    loop {
                        match self.ch.recv() {
                            Ok(CMessage::Shutdown) => {
                                return Err(());
                            }
                            Ok(CMessage::Response(r)) => {
                                resp = r;
                                break;
                            }
                            Err(_) => {
                                thread::sleep(time::Duration::from_millis(3));
                            }
                        }
                    }
                    serde_json::to_string(&resp).unwrap()
                } else {
                    serde_json::to_string(&Response::Err("Shutting down!".to_owned())).unwrap()
                }
            }
            Err(e) => serde_json::to_string(&Response::Err(e)).unwrap(),
        };
        debug!(self.l, "Request handled!");
        let mut resp = tiny_http::Response::from_string(resp);
        let cors_o = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
        let cors_m = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, GET"[..]).unwrap();
        let cors_h = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap();
        resp.add_header(cors_o);
        resp.add_header(cors_m);
        resp.add_header(cors_h);
        request.respond(resp).unwrap();
        Ok(())
    }
    */
}
