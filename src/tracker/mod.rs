mod http;
mod udp;

use std::sync::mpsc;
use byteorder::{BigEndian, ReadBytesExt};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::thread;
use slog::Logger;
use torrent::Torrent;
use bencode::BEncode;
use url::Url;
use {CONTROL, CONFIG, TC};

pub struct Tracker {
    queue: mpsc::Receiver<Request>,
    http: http::Announcer,
    udp: udp::Announcer,
    l: Logger,
}

impl Tracker {
    pub fn new(queue: mpsc::Receiver<Request>, l: Logger) -> Tracker {
        Tracker {
            queue,
            http: http::Announcer::new(),
            udp: udp::Announcer::new(),
            l,
        }
    }

    pub fn run(&mut self) {
        debug!(self.l, "Initialized!");
        loop {
            match self.queue.recv() {
                Ok(Request::Announce(req)) => {
                    debug!(self.l, "Handling announce request!");
                    let stopping = req.stopping();
                    let id = req.id;
                    let response = if let Ok(url) = Url::parse(&req.url) {
                        match url.scheme() {
                            "http" => self.http.announce(req),
                            "udp" => self.udp.announce(req),
                            _ => Err(TrackerError::InvalidURL),
                        }
                    } else {
                        Err(TrackerError::InvalidURL)
                    };
                    if !stopping {
                        debug!(self.l, "Sending announce response to control!");
                        if CONTROL.trk_tx.lock().unwrap().send((id, response)).is_err() {
                        }
                    }
                }
                Ok(Request::Shutdown) => {
                    debug!(self.l, "Shutdown!");
                    break;
                }
                _ => { unreachable!() }
            }
        }
    }
}


pub struct Handle {
    pub tx: mpsc::Sender<Request>,
}

impl Handle {
    pub fn init(&self) { }

    pub fn get(&self) -> mpsc::Sender<Request> {
        self.tx.clone()
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

pub type Response = (usize, TrackerRes);
pub type TrackerRes = Result<TrackerResponse, TrackerError>;

#[derive(Clone, Debug, Serialize)]
pub enum TrackerError {
    Error(String),
    InvalidURL,
    ConnectionFailure,
    InvalidResponse(&'static str),
}

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

    pub fn from_bencode(data: BEncode) -> TrackerRes {
        let mut d = data.to_dict().ok_or(TrackerError::InvalidResponse("Tracker response must be a dictionary type!"))?;
        if let Some(BEncode::String(data)) = d.remove("failure reason") {
            return Err(TrackerError::Error(String::from_utf8(data).map_err(|_| TrackerError::InvalidResponse("Failure reason must be UTF8!"))?));
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
                return Err(TrackerError::InvalidResponse("Response must have peers field!"));
            }
        };
        match d.remove("interval") {
            Some(BEncode::Int(ref i)) => {
                resp.interval = *i as u32;
            }
            _ => {
                return Err(TrackerError::InvalidResponse("Response must have interval!"));
            }
        };
        Ok(resp)
    }
}

pub fn start(l: Logger) -> Handle {
    debug!(l, "Initializing!");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut d = Tracker::new(rx, l);
        d.run();
        use std::sync::atomic;
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
    });
    Handle { tx }
}
