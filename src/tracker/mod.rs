mod http;
mod udp;
mod errors;
mod dns;
mod dht;

use byteorder::{BigEndian, ReadBytesExt};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::{result, io};
use slog::Logger;
use torrent::Torrent;
use bencode::BEncode;
use url::Url;
use {CONFIG, LOG};
use control::cio;
use handle;
use amy;
pub use self::errors::{Result, ResultExt, Error, ErrorKind};

pub struct Tracker {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    dns_res: amy::Receiver<dns::QueryResponse>,
    http: http::Handler,
    udp: udp::Handler,
    dht: dht::Manager,
    dns: dns::Resolver,
    timer: usize,
    l: Logger,
    shutting_down: bool,
}

#[derive(Debug)]
pub enum Request {
    Announce(Announce),
    GetPeers(GetPeers),
    AddNode(SocketAddr),
    DHTAnnounce([u8; 20]),
    Shutdown,
}

#[derive(Debug)]
pub struct Announce {
    id: usize,
    url: String,
    hash: [u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
    event: Option<Event>,
}

#[derive(Debug)]
pub struct GetPeers {
    pub id: usize,
    pub hash: [u8; 20],
}


#[derive(Debug)]
pub enum Event {
    Started,
    Stopped,
    Completed,
}

pub type Response = (usize, Result<TrackerResponse>);

#[derive(Debug)]
pub struct TrackerResponse {
    pub peers: Vec<SocketAddr>,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
}


impl Tracker {
    pub fn new(
        poll: amy::Poller,
        ch: handle::Handle<Request, Response>,
        udp: udp::Handler,
        dht: dht::Manager,
        http: http::Handler,
        dns: dns::Resolver,
        dns_res: amy::Receiver<dns::QueryResponse>,
        timer: usize,
        l: Logger)
        -> Tracker {
        Tracker {
            ch,
            http,
            udp,
            dht,
            l,
            poll,
            dns,
            dns_res,
            timer,
            shutting_down: false,
        }
    }

    pub fn run(&mut self) {
        self.dht.init();

        debug!(self.l, "Initialized!");
        'outer: loop {
            for event in self.poll.wait(10).unwrap() {
                if self.handle_event(event).is_err() {
                    break 'outer;
                }
            }
        }

        debug!(self.l, "Shutting down!");
        self.shutting_down = true;

        // Shutdown loop - wait for all requests to complete
        loop {
            for event in self.poll.wait(50).unwrap() {
                if self.handle_event(event).is_err() {
                }
                if self.http.complete() && self.udp.complete() {
                    return;
                }
            }
        }
    }

    fn handle_event(&mut self, event: amy::Notification)  -> result::Result<(), ()> {
        if event.id == self.ch.rx.get_id() {
            return self.handle_request();
        } else if event.id == self.dns_res.get_id() {
            self.handle_dns_res();
        } else if event.id == self.timer {
            self.handle_timer();
        } else {
            self.handle_socket(event);
        }
        Ok(())
    }

    fn handle_request(&mut self) -> result::Result<(), ()> {
        while let Ok(r) = self.ch.recv() {
            match r {
                Request::Announce(req) => {
                    debug!(self.l, "Handling announce request!");
                    let id = req.id;
                    let response = if let Ok(url) = Url::parse(&req.url) {
                        match url.scheme() {
                            "http" => self.http.new_announce(req, &url, &mut self.dns),
                            "udp" => self.udp.new_announce(req, &url, &mut self.dns),
                            s => Err(ErrorKind::InvalidRequest(format!("Unknown tracker url scheme: {}", s)).into()),
                        }
                    } else {
                        Err(ErrorKind::InvalidRequest(format!("Invalid url: {}", req.url)).into())
                    };
                    if let Err(e) = response {
                        self.send_response((id, Err(e)));
                    }
                }
                Request::GetPeers(gp) => {
                    debug!(self.l, "Handling dht peer find req!");
                    self.dht.get_peers(gp.id, gp.hash);
                }
                Request::AddNode(addr) => {
                    debug!(self.l, "Handling dht node addition req!");
                    self.dht.add_addr(addr);
                }
                Request::DHTAnnounce(hash) => {
                    debug!(self.l, "Handling dht node addition req!");
                    self.dht.announce(hash);
                }
                Request::Shutdown => {
                    return Err(());
                }
            }
        }
        Ok(())
    }

    fn handle_dns_res(&mut self) {
        while let Ok(r) = self.dns_res.try_recv() {
            let resp = if self.http.contains(r.id) {
                self.http.dns_resolved(r)
            } else if self.udp.contains(r.id) {
                self.udp.dns_resolved(r)
            } else {
                None
            };
            if let Some(r) = resp {
                self.send_response(r);
            }
        }
    }

    fn handle_timer(&mut self) {
        for r in self.http.tick() {
            self.send_response(r);
        }

        for r in self.udp.tick() {
            self.send_response(r);
        }

        self.dns.tick();
        self.dht.tick();
    }


    fn handle_socket(&mut self, event: amy::Notification) {
        if self.http.contains(event.id) {
            let resp = if event.event.readable() {
                self.http.readable(event.id)
            } else {
                self.http.writable(event.id)
            };
            if let Some(r) = resp {
                self.send_response(r);
            }
        } else if self.udp.id() == event.id {
            for resp in self.udp.readable() {
                self.send_response(resp);
            }
        } else if self.dht.id() == event.id {
            for resp in self.dht.readable() {
                self.send_response(resp);
            }
        } else if self.dns.contains(event.id) {
            if event.event.readable() {
                self.dns.readable(event.id);
            } else {
                self.dns.writable(event.id);
            }
        } else {
            unreachable!();
        };

    }

    fn send_response(&self, r: Response) {
        if !self.shutting_down {
            debug!(self.l, "Sending trk response to control!");
            self.ch.send(r).unwrap();
        }
    }
}

impl Request {
    pub fn new_announce<T: cio::CIO>(torrent: &Torrent<T>, event: Option<Event>) -> Request {
        Request::Announce(Announce {
            id: torrent.id(),
            url: torrent.info().announce.clone(),
            hash: torrent.info().hash,
            port: CONFIG.port,
            uploaded: torrent.uploaded() as u64 * torrent.info().piece_len as u64,
            downloaded: torrent.downloaded() as u64 * torrent.info().piece_len as u64,
            left: torrent.info().total_len - torrent.downloaded() as u64 * torrent.info().piece_len as u64,
            event,
        })
    }

    pub fn started<T: cio::CIO>(torrent: &Torrent<T>) -> Request {
        Request::new_announce(torrent, Some(Event::Started))
    }

    pub fn stopped<T: cio::CIO>(torrent: &Torrent<T>) -> Request {
        Request::new_announce(torrent, Some(Event::Stopped))
    }

    pub fn completed<T: cio::CIO>(torrent: &Torrent<T>) -> Request {
        Request::new_announce(torrent, Some(Event::Completed))
    }

    pub fn interval<T: cio::CIO>(torrent: &Torrent<T>) -> Request {
        Request::new_announce(torrent, None)
    }
}

impl TrackerResponse {
    pub fn empty() -> TrackerResponse {
        TrackerResponse {
            peers: vec![],
            interval: 900,
            leechers: 0,
            seeders: 0,
        }
    }

    pub fn from_bencode(data: BEncode) -> Result<TrackerResponse> {
        let mut d = data.to_dict()
            .ok_or(ErrorKind::InvalidResponse("Tracker response must be a dictionary type!"))?;
        if let Some(BEncode::String(data)) = d.remove("failure reason") {
            let reason = String::from_utf8(data).chain_err(|| ErrorKind::InvalidResponse("Failure reason must be UTF8!"))?;
            return Err(ErrorKind::TrackerError(reason).into());
        }
        let mut resp = TrackerResponse::empty();
        match d.remove("peers") {
            Some(BEncode::String(ref data)) => {
                for p in data.chunks(6) {
                    let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
                    let socket = SocketAddrV4::new(ip, (&p[4..]).read_u16::<BigEndian>().unwrap());
                    resp.peers.push(SocketAddr::V4(socket));
                }
            }
            _ => {
                return Err(ErrorKind::InvalidResponse("Response must have peers field!").into());
            }
        };
        match d.remove("interval") {
            Some(BEncode::Int(ref i)) => {
                resp.interval = *i as u32;
            }
            _ => {
                return Err(ErrorKind::InvalidResponse("Response must have interval!").into());
            }
        };
        Ok(resp)
    }
}

pub fn start(creg: &mut amy::Registrar) -> io::Result<handle::Handle<Response, Request>> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar()?;
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    let timer = reg.set_interval(150)?;
    let (dtx, drx) = reg.channel()?;
    let udp = udp::Handler::new(&reg, LOG.new(o!("trk" => "udp")))?;
    let dht = dht::Manager::new(&reg, LOG.new(o!("trk" => "dht")))?;
    let http = http::Handler::new(&reg, LOG.new(o!("trk" => "http")))?;
    let dns = dns::Resolver::new(reg, dtx);
    dh.run("trk", move |h, l| Tracker::new(poll, h, udp, dht, http, dns, drx, timer, l).run());
    Ok(ch)
}
