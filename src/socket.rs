use std::net::{TcpStream, SocketAddr};
use std::os::unix::io::{RawFd, AsRawFd};
use std::io::{self, ErrorKind};
use std::mem;

use net2::{TcpBuilder, TcpStreamExt};
use openssl::ssl::{SslConnectorBuilder, SslMethod, MidHandshakeSslStream, SslStream,
                   HandshakeError};
use amy;

use throttle::Throttle;
use util;

const EINPROGRESS: i32 = 115;

/// Wrapper type over Mio sockets, allowing for use of UDP/TCP, encryption,
/// rate limiting, etc.
pub struct Socket {
    conn: TcpStream,
    addr: SocketAddr,
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
        if let Err(e) = conn.connect(addr) {
            if Some(EINPROGRESS) != e.raw_os_error() {
                return Err(e);
            }
        }
        Ok(Socket {
            conn,
            throttle: None,
            addr: *addr,
        })
    }

    #[cfg(test)]
    pub fn empty() -> Socket {
        let conn = TcpBuilder::new_v4().unwrap().to_tcp_stream().unwrap();
        Socket {
            conn,
            throttle: None,
            addr: "127.0.0.1:0".parse().unwrap(),
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn from_stream(conn: TcpStream) -> io::Result<Socket> {
        conn.set_nonblocking(true)?;
        let addr = conn.peer_addr().unwrap();
        Ok(Socket {
            conn,
            throttle: None,
            addr: addr,
        })
    }

    pub fn peek(&self, buf: &mut [u8]) -> io::Result<usize> {
        self.conn.peek(buf)
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

pub struct TSocket {
    conn: TConn,
    fd: i32,
    reg: amy::Registrar,
}

enum TConn {
    Empty,
    Plain(TcpStream),
    // SSL Preconnection state
    SSLP { host: String, conn: TcpStream },
    // SSL Connecting state
    SSLC(MidHandshakeSslStream<TcpStream>),
    SSL(SslStream<TcpStream>),
}

impl TSocket {
    pub fn new_v4(r: &amy::Registrar, host: Option<String>) -> io::Result<(usize, TSocket)> {
        let reg = r.try_clone()?;
        let conn = TcpBuilder::new_v4()?.to_tcp_stream()?;
        conn.set_nonblocking(true)?;
        let id = reg.register(&conn, amy::Event::Both)?;
        let fd = conn.as_raw_fd();
        let sock = match host {
            Some(h) => TSocket {
                reg,
                conn: TConn::SSLP { host: h, conn },
                fd,
            },
            None => TSocket {
                reg,
                conn: TConn::Plain(conn),
                fd,
            },
        };
        Ok((id, sock))
    }

    pub fn ssl(&self) -> bool {
        match self.conn {
            TConn::Plain(_) => false,
            _ => true,
        }
    }

    pub fn connect(&mut self, addr: SocketAddr) -> io::Result<()> {
        let c = mem::replace(&mut self.conn, TConn::Empty);
        self.conn = match c {
            TConn::Plain(c) => {
                if let Err(e) = c.connect(addr) {
                    if Some(EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                TConn::Plain(c)
            }
            TConn::SSLP { host, conn } => {
                if let Err(e) = conn.connect(addr) {
                    if Some(EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                let connector = if let Ok(b) = SslConnectorBuilder::new(SslMethod::tls()) {
                    b.build()
                } else {
                    return util::io_err("SSL Connection failed!");
                };
                match connector.connect(&host, conn) {
                    Ok(s) => TConn::SSL(s),
                    Err(HandshakeError::Interrupted(s)) => TConn::SSLC(s),
                    Err(_) => return util::io_err("SSL Connection failed!"),
                }
            }
            _ => return util::io_err("Socket in failed state!"),
        };
        Ok(())
    }
}

impl io::Read for TSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let c = mem::replace(&mut self.conn, TConn::Empty);
        let res;
        self.conn = match c {
            TConn::Plain(mut c) => {
                res = c.read(buf);
                TConn::Plain(c)
            }
            TConn::SSLC(conn) => {
                match conn.handshake() {
                    Ok(s) => {
                        res = Ok(::std::usize::MAX);
                        TConn::SSL(s)
                    }
                    Err(HandshakeError::Interrupted(s)) => {
                        res = Ok(0);
                        TConn::SSLC(s)
                    }
                    Err(_) => {
                        res = util::io_err("SSL Connection failed!");
                        TConn::Empty
                    }
                }
            }
            TConn::SSL(mut conn) => {
                res = conn.read(buf);
                TConn::SSL(conn)
            }
            _ => return util::io_err("Socket in failed state!"),
        };

        res
    }
}

impl io::Write for TSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let c = mem::replace(&mut self.conn, TConn::Empty);
        let res;
        self.conn = match c {
            TConn::Plain(mut c) => {
                res = c.write(buf);
                TConn::Plain(c)
            }
            TConn::SSLC(conn) => {
                match conn.handshake() {
                    Ok(s) => {
                        res = Ok(::std::usize::MAX);
                        TConn::SSL(s)
                    }
                    Err(HandshakeError::Interrupted(s)) => {
                        res = Ok(0);
                        TConn::SSLC(s)
                    }
                    Err(_) => return util::io_err("SSL Connection failed!"),
                }
            }
            TConn::SSL(mut conn) => {
                res = conn.write(buf);
                TConn::SSL(conn)
            }
            _ => return util::io_err("Socket in failed state!"),
        };

        res
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.conn {
            TConn::Plain(ref mut c) => c.flush(),
            TConn::SSL(ref mut c) => c.flush(),
            _ => Ok(()),
        }
    }
}

impl AsRawFd for TSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}


impl Drop for TSocket {
    fn drop(&mut self) {
        if self.reg.deregister(&*self).is_err() {
            // TODO: idk? does it matter?
        }
    }
}
