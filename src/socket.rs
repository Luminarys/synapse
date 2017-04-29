use mio::net::TcpStream;
use std::io;
use iovec::IoVec;

/// Wrapper type over Mio sockets, allowing for use of UDP/TCP, encryption,
/// rate limiting, etc.
pub struct Socket {
    conn: TcpStream,
}

impl Socket {
    pub fn new(conn: TcpStream) -> Socket {
        Socket { conn }
    }
}

impl io::Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut b: [&mut IoVec; 1] = [buf.into()];
        self.conn.read_bufs(&mut b)
    }
}

impl io::Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut b: [&IoVec; 1] = [buf.into()];
        self.conn.write_bufs(&b)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.conn.flush()
    }
}
