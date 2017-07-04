mod http;
mod udp;
mod errors;

use std::rc::Rc;
use byteorder::{BigEndian, ReadBytesExt};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::thread;
use std::result;
use slog::Logger;
use torrent::Torrent;
use bencode::BEncode;
use url::Url;
use {CONTROL, CONFIG, TC};
use amy;
pub use self::errors::{Result, ResultExt, Error, ErrorKind};

pub struct Tracker {
    poll: amy::Poller,
    reg: Rc<amy::Registrar>,
    queue: amy::Receiver<Request>,
    http: http::Announcer,
    udp: udp::Announcer,
    l: Logger,
}

impl Tracker {
    pub fn new(poll: amy::Poller, reg: amy::Registrar, queue: amy::Receiver<Request>, l: Logger) -> Tracker {
        let reg = Rc::new(reg);
        Tracker {
            queue,
            http: http::Announcer::new(reg.clone()),
            udp: udp::Announcer::new(reg.clone()),
            l,
            poll,
            reg,
        }
    }

    pub fn run(&mut self) {
        debug!(self.l, "Initialized!");
        loop {
            for event in self.poll.wait(3).unwrap() {
                // TODO: Handle lingering connections.
                if self.handle_event(event).is_err() {
                    break;
                }
            }
        }
    }

    fn handle_event(&mut self, event: amy::Notification)  -> result::Result<(), ()> {
        if event.id == self.queue.get_id() {
            self.handle_request()
        } else {
            self.handle_socket(event);
            Ok(())
        }
    }

    fn handle_request(&mut self) -> result::Result<(), ()> {
        while let Ok(r) = self.queue.try_recv() {
            match r {
                Request::Announce(req) => {
                    debug!(self.l, "Handling announce request!");
                    let id = req.id;
                    let stopping = req.stopping();
                    let response = if let Ok(url) = Url::parse(&req.url) {
                        match url.scheme() {
                            "http" => self.http.new_announce(req),
                            "udp" => self.udp.new_announce(req),
                            s => Some(Err(ErrorKind::InvalidRequest(format!("Unknown tracker url scheme: {}", s)).into())),
                        }
                    } else {
                        Some(Err(ErrorKind::InvalidRequest(format!("Invalid url: {}", req.url)).into()))
                    };
                    if !stopping {
                        if let Some(Err(e)) = response {
                            debug!(self.l, "Sending announce response to control!");
                            CONTROL.trk_tx.lock().unwrap().send((id, Err(e))).unwrap();
                        }
                    }
                }
                Request::Shutdown => {
                    debug!(self.l, "Shutdown!");
                    return Err(());
                }
            }
        }
        Ok(())
    }

    fn handle_socket(&mut self, event: amy::Notification) {
        let resp = if self.http.contains(event.id) {
            if event.event.readable() {
                self.http.readable(event.id)
            } else {
                self.http.writable(event.id)
            }
        } else if self.udp.contains(event.id) {
            if event.event.readable() {
                self.udp.readable(event.id)
            } else {
                self.udp.writable(event.id)
            }
        } else {
            unreachable!();
        };

        if let Some(r) = resp {
            debug!(self.l, "Sending announce response to control!");
            CONTROL.trk_tx.lock().unwrap().send(r).unwrap();
        }
    }
}


pub struct Handle {
    pub tx: amy::Sender<Request>,
}

impl Handle {
    pub fn init(&self) { }

    pub fn get(&self) -> amy::Sender<Request> {
        self.tx.try_clone().unwrap()
    }
}

unsafe impl Sync for Handle {}


#[derive(Debug)]
pub enum Request {
    Announce(Announce),
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

impl Announce {
    pub fn stopping(&self) -> bool {
        match self.event {
            Some(Event::Stopped) => true,
            _ => false,
        }
    }
}

impl Request {
    pub fn new_announce(torrent: &Torrent, event: Option<Event>) -> Request {
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

    pub fn started(torrent: &Torrent) -> Request {
        Request::new_announce(torrent, Some(Event::Started))
    }

    pub fn stopped(torrent: &Torrent) -> Request {
        Request::new_announce(torrent, Some(Event::Stopped))
    }

    pub fn completed(torrent: &Torrent) -> Request {
        Request::new_announce(torrent, Some(Event::Completed))
    }

    pub fn interval(torrent: &Torrent) -> Request {
        Request::new_announce(torrent, None)
    }
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

pub fn start(l: Logger) -> Handle {
    debug!(l, "Initializing!");
    let p = amy::Poller::new().unwrap();
    let mut r = p.get_registrar().unwrap();
    let (tx, rx) = r.channel().unwrap();
    thread::spawn(move || {
        let mut d = Tracker::new(p, r, rx, l);
        d.run();
        use std::sync::atomic;
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
    });
    Handle { tx }
}
