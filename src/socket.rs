use std::net::{TcpStream, SocketAddr};
use std::os::unix::io::{RawFd, AsRawFd};
use std::io::{self, ErrorKind};
use throttle::Throttle;
use net2::{TcpBuilder, TcpStreamExt};

/// Wrapper type over Mio sockets, allowing for use of UDP/TCP, encryption,
/// rate limiting, etc.
pub struct Socket {
    conn: TcpStream,
    pub throttle: Option<Throttle>,
}

impl Socket {
    pub fn new(addr: &SocketAddr) -> io::Result<Socket> {
        let sock = (match *addr {
            SocketAddr::V4(..) => TcpBuilder::new_v4(),
            SocketAddr::V6(..) => TcpBuilder::new_v6(),
        })?;
        let conn = sock.to_tcp_stream()?;
        conn.set_nonblocking(true)?;
        // TODO: Need to reliably check this.
        conn.connect(addr);
        Ok(Socket { conn, throttle: None })
    }

    pub fn from_stream(conn: TcpStream) -> io::Result<Socket> {
        conn.set_nonblocking(true)?;
        Ok(Socket { conn, throttle: None })
    }
}

impl AsRawFd for Socket {
    fn as_raw_fd(&self) -> RawFd {
        self.conn.as_raw_fd()
    }
 }

impl io::Read for Socket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // Don't bother rate limiting small requests
        if buf.len() < 20 {
            return self.conn.read(buf);
        }
        if let Some(ref mut t) = self.throttle {
            match t.get_bytes_dl(buf.len()) {
                Ok(()) => {
                    match self.conn.read(buf) {
                        Ok(amnt) => { t.restore_bytes_dl(buf.len() - amnt); Ok(amnt) }
                        Err(e) => {
                            t.restore_bytes_dl(buf.len());
                            Err(e)
                        }
                    }
                }
                Err(()) => {
                    Err(io::Error::new(ErrorKind::WouldBlock, ""))
                }
            }
        } else {
            self.conn.read(buf)
        }
    }
}

impl io::Write for Socket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() < 20 {
            return self.conn.write(buf);
        }
        if let Some(ref mut t) = self.throttle {
            match t.get_bytes_ul(buf.len()) {
                Ok(()) => {
                    match self.conn.write(buf) {
                        Ok(amnt) => { t.restore_bytes_ul(buf.len() - amnt); Ok(amnt) }
                        Err(e) => {
                            t.restore_bytes_ul(buf.len());
                            Err(e)
                        }
                    }
                }
                Err(()) => {
                    Err(io::Error::new(ErrorKind::WouldBlock, ""))
                }
            }
        } else {
            self.conn.write(buf)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.conn.flush()
    }
}
