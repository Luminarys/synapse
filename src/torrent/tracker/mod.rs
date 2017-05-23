mod http;
mod udp;

use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::io;
use bencode::BEncode;
use byteorder::{BigEndian, ReadBytesExt};

/// A tracker represents a connection to a bittorrent tracker
/// and associated metadata.
pub struct Tracker {
    interval: u32,
    peers: Vec<SocketAddr>,
    conn: TrackerConn,
}

pub enum TrackerConn {
    HTTP(http::HttpTracker),
    UDP(udp::UdpTracker),
}

#[derive(Debug)]
pub struct Request {
    hash: [u8; 20],
    port: u16,
    uploaded: usize,
    downloaded: usize,
    left: usize,
    event: Event,
}

#[derive(Debug)]
pub enum Event {
    Started,
    Stopped,
    Completed,
}


#[derive(Debug)]
pub struct Response {
    pub peers: Vec<SocketAddr>,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
}

impl Tracker {
    pub fn new_http() -> io::Result<Tracker> {
        unimplemented!();
    }

    pub fn new_udp() -> io::Result<Tracker> {
        unimplemented!();
    }

    pub fn new_request(&mut self, req: Request) -> io::Result<()> {
        Ok(())
    }

    pub fn readable(&mut self) -> io::Result<Option<Response>> {
        Ok(None)
    }

    pub fn writable(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Response {
    pub fn empty() -> Response {
        Response {
            peers: vec![],
            interval: 900,
            leechers: 0,
            seeders: 0,
        }
    }

    pub fn from_bencode(data: BEncode) -> Result<Response, String> {
        let mut d = data.to_dict().ok_or("File must be a dictionary type!".to_string())?;
        let mut resp = Response::empty();
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
