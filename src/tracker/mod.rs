mod http;
mod udp;

use std::sync::{mpsc, atomic};
use byteorder::{BigEndian, ReadBytesExt};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::thread;
use {CONTROL, PORT};
use torrent::Torrent;
use bencode::BEncode;
use url::Url;

pub struct Tracker {
    queue: mpsc::Receiver<Request>,
    http: http::Announcer,
    udp: udp::Announcer,
}

impl Tracker {
    pub fn new(queue: mpsc::Receiver<Request>) -> Tracker {
        Tracker {
            queue,
            http: http::Announcer::new(),
            udp: udp::Announcer::new(),
        }
    }

    pub fn run(&mut self) {
        loop {
            if let Ok(req) = self.queue.recv() {
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
                CONTROL.trk_tx.lock().unwrap().send((id, response)).unwrap();
            } else {
                break;
            }
        }
    }
}


pub struct Handle {
    pub tx: mpsc::Sender<Request>,
}

impl Handle {
    pub fn get(&self) -> mpsc::Sender<Request> {
        self.tx.clone()
    }
}

unsafe impl Sync for Handle {}

#[derive(Debug)]
pub struct Request {
    id: usize,
    url: String,
    hash: [u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
    event: Option<Event>,
}

impl Request {
    pub fn new(torrent: &Torrent, event: Option<Event>) -> Request {
        Request {
            id: torrent.id,
            url: torrent.info.announce.clone(),
            hash: torrent.info.hash,
            port: PORT.load(atomic::Ordering::Relaxed) as u16,
            uploaded: torrent.uploaded as u64 * torrent.info.piece_len as u64,
            downloaded: torrent.downloaded as u64 * torrent.info.piece_len as u64,
            left: torrent.info.total_len - torrent.downloaded as u64 * torrent.info.piece_len as u64,
            event,
        }
    }

    pub fn started(torrent: &Torrent) -> Request {
        Request::new(torrent, Some(Event::Started))
    }

    pub fn stopped(torrent: &Torrent) -> Request {
        Request::new(torrent, Some(Event::Started))
    }

    pub fn completed(torrent: &Torrent) -> Request {
        Request::new(torrent, Some(Event::Completed))
    }

    pub fn interval(torrent: &Torrent) -> Request {
        Request::new(torrent, None)
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
        match d.remove("failure_reason") {
            Some(BEncode::String(data)) => {
                return Err(TrackerError::Error(String::from_utf8(data).map_err(|_| TrackerError::InvalidResponse("Failure reason must be UTF8!"))?));
            }
            _ => { }
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

pub fn start() -> Handle {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut d = Tracker::new(rx);
        d.run();
    });
    Handle { tx }
}
