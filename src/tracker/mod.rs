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
                let url = Url::parse(&req.url).unwrap();
                let response = match url.scheme() {
                    "http" => self.http.announce(req),
                    "udp" => self.udp.announce(req),
                    _ => unreachable!(),
                };
                CONTROL.trk_tx.lock().unwrap().send(response).unwrap();
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
    uploaded: usize,
    downloaded: usize,
    left: usize,
    event: Option<Event>,
}

impl Request {
    pub fn new(torrent: &Torrent, event: Option<Event>) -> Request {
        Request {
            id: torrent.id,
            url: torrent.info.announce.clone(),
            hash: torrent.info.hash,
            port: PORT.load(atomic::Ordering::Relaxed) as u16,
            uploaded: torrent.uploaded,
            downloaded: torrent.downloaded,
            left: torrent.info.total_len - torrent.downloaded,
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

#[derive(Debug)]
pub struct Response {
    pub id: usize,
    pub peers: Vec<SocketAddr>,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
}

impl Response {
    pub fn empty(id: usize) -> Response {
        Response {
            id,
            peers: vec![],
            interval: 900,
            leechers: 0,
            seeders: 0,
        }
    }

    pub fn from_bencode(id: usize, data: BEncode) -> Result<Response, String> {
        let mut d = data.to_dict().ok_or("File must be a dictionary type!".to_string())?;
        let mut resp = Response::empty(id);
        match d.remove("peers") {
            Some(BEncode::String(ref data)) => {
                for p in data.chunks(6) {
                    let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
                    let socket = SocketAddrV4::new(ip, (&p[4..]).read_u16::<BigEndian>().unwrap());
                    resp.peers.push(SocketAddr::V4(socket));
                }
            }
            _ => {
                return Err("Response must have peers!".to_string());
            }
        };
        match d.remove("interval") {
            Some(BEncode::Int(ref i)) => {
                resp.interval = *i as u32;
            }
            _ => {
                return Err("Response must have interval!".to_string());
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
