use std::sync::mpsc;
use byteorder::{BigEndian, ReadBytesExt};
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::thread;
use util::{encode_param, append_pair};
use {PEER_ID, CONTROL, reqwest, bencode};
use bencode::BEncode;
use torrent::Torrent;

pub struct Tracker {
    queue: mpsc::Receiver<Request>,
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
    event: Event,
}

impl Request {
    pub fn new(id: usize, port: u16, torrent: &Torrent, event: Event) -> Request {
        Request {
            id,
            url: torrent.info.announce.clone(),
            hash: torrent.info.hash,
            port,
            uploaded: torrent.uploaded,
            downloaded: torrent.downloaded,
            left: torrent.info.total_len - torrent.downloaded,
            event,
        }
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

impl Tracker {
    pub fn new(queue: mpsc::Receiver<Request>) -> Tracker {
        Tracker {
            queue
        }
    }

    pub fn run(&mut self) {
        loop {
            if let Ok(mut req) = self.queue.recv() {
                let mut url = &mut req.url;
                // The fact that I have to do this is genuinely depressing.
                // This will be rewritten as a proper http protocol
                // encoder in the event loop eventually.
                url.push_str("?");
                append_pair(&mut url, "info_hash", &encode_param(&req.hash));
                append_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
                append_pair(&mut url, "uploaded", &req.uploaded.to_string());
                append_pair(&mut url, "numwant", "25");
                append_pair(&mut url, "downloaded", &req.downloaded.to_string());
                append_pair(&mut url, "left", &req.left.to_string());
                append_pair(&mut url, "compact", "1");
                append_pair(&mut url, "port", &req.port.to_string());
                let response = {
                    let mut resp = reqwest::get(&*url).unwrap();
                    let content = bencode::decode(&mut resp).unwrap();
                    Response::from_bencode(req.id, content).unwrap()
                };
                CONTROL.trk_tx.send(response).unwrap();
            } else {
                break;
            }
        }
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
