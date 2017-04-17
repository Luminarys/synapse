use mio::tcp::TcpStream;
use std::net::SocketAddr;
use std::io;

pub struct Tracker {
    conn: TcpStream,
    id: Option<String>,
    interval: u32,
    peers: Vec<SocketAddr>,
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
