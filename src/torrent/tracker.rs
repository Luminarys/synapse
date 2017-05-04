use mio::tcp::TcpStream;
use std::net::{SocketAddr, SocketAddrV4, Ipv4Addr};
use std::io;
use bencode::BEncode;
use byteorder::{BigEndian, ReadBytesExt};

pub struct Tracker {
    conn: TcpStream,
    id: Option<Vec<u8>>,
    interval: u32,
    peers: Vec<SocketAddr>,
}

#[derive(Debug)]
pub struct Response {
    pub peers: Vec<SocketAddr>,
    pub interval: u32,
    pub id: Option<Vec<u8>>,
}

impl Tracker {
    pub fn new() -> io::Result<Tracker> {
        unimplemented!();
    }

    pub fn readable(&mut self) {
    
    }

    pub fn writable(&mut self) {
    
    }
}

impl Response {
    pub fn from_bencode(data: BEncode) -> Result<Response, String> {
        let mut d = data.to_dict().ok_or("File must be a dictionary type!".to_string())?;
        let peers = Vec::new();
        let mut resp = Response {
            peers: peers,
            interval: 900,
            id: None,
        };
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
        match d.remove("tracker id") {
            Some(BEncode::String(ref s)) => {
                resp.id = Some(s.clone());
            }
            _ => ()
        };
        Ok(resp)
    }
}
