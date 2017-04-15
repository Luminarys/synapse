use mio::tcp::TcpStream;
use std::io;

pub struct Announcer {
    conn: TcpStream,
}

impl Announcer {
    pub fn new() -> io::Result<Announcer> {
        unimplemented!();
    }

    pub fn readable(&mut self) {
    
    }

    pub fn writable(&mut self) {
    
    }
}
