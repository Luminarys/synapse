use std::io::{self, ErrorKind};
use std::mem;
use std::net::{SocketAddr, TcpStream};
use std::os::unix::io::{AsRawFd, RawFd};

use net2::{TcpBuilder, TcpStreamExt};
use nix::libc;
use openssl::ssl::{
    HandshakeError, MidHandshakeSslStream, SslAcceptor, SslConnector, SslMethod, SslStream,
};

use throttle::Throttle;
use util;

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
            // OSX gives the AddrNotAvailable error sometimes
            if Some(libc::EINPROGRESS) != e.raw_os_error()
                && e.kind() != ErrorKind::AddrNotAvailable
            {
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
        let addr = conn.peer_addr()?;
        Ok(Socket {
            conn,
            throttle: None,
            addr,
        })
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
                Ok(()) => match self.conn.read(buf) {
                    Ok(amnt) => {
                        t.restore_bytes_dl(buf.len() - amnt);
                        Ok(amnt)
                    }
                    Err(e) => {
                        t.restore_bytes_dl(buf.len());
                        Err(e)
                    }
                },
                Err(()) => Err(io::Error::new(ErrorKind::WouldBlock, "")),
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
                Ok(()) => match self.conn.write(buf) {
                    Ok(amnt) => {
                        t.restore_bytes_ul(buf.len() - amnt);
                        Ok(amnt)
                    }
                    Err(e) => {
                        t.restore_bytes_ul(buf.len());
                        Err(e)
                    }
                },
                Err(()) => Err(io::Error::new(ErrorKind::WouldBlock, "")),
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
    pub fn new_v4(host: Option<String>) -> io::Result<TSocket> {
        let conn = TcpBuilder::new_v4()?.to_tcp_stream()?;
        conn.set_nonblocking(true)?;
        let fd = conn.as_raw_fd();
        let sock = match host {
            Some(h) => TSocket {
                conn: TConn::SSLP { host: h, conn },
                fd,
            },
            None => TSocket {
                conn: TConn::Plain(conn),
                fd,
            },
        };
        Ok(sock)
    }

    pub fn from_plain(stream: TcpStream) -> io::Result<TSocket> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(TSocket {
            conn: TConn::Plain(stream),
            fd,
        })
    }

    pub fn from_ssl(stream: TcpStream, acceptor: &SslAcceptor) -> io::Result<TSocket> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();

        let conn = match acceptor.accept(stream) {
            Ok(c) => TConn::SSL(c),
            Err(HandshakeError::WouldBlock(s)) => TConn::SSLC(s),
            Err(_) => return util::io_err("SSL Connection failed!"),
        };
        Ok(TSocket { conn, fd })
    }

    pub fn connect(&mut self, addr: SocketAddr) -> io::Result<()> {
        let c = mem::replace(&mut self.conn, TConn::Empty);
        self.conn = match c {
            TConn::Plain(c) => {
                if let Err(e) = c.connect(addr) {
                    if Some(libc::EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                TConn::Plain(c)
            }
            TConn::SSLP { host, conn } => {
                if let Err(e) = conn.connect(addr) {
                    if Some(libc::EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                let connector = if let Ok(b) = SslConnector::builder(SslMethod::tls()) {
                    b.build()
                } else {
                    return util::io_err("SSL Connection failed!");
                };
                match connector.connect(&host, conn) {
                    Ok(s) => TConn::SSL(s),
                    Err(HandshakeError::WouldBlock(s)) => TConn::SSLC(s),
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
            TConn::SSLC(conn) => match conn.handshake() {
                Ok(s) => {
                    res = Ok(::std::usize::MAX);
                    TConn::SSL(s)
                }
                Err(HandshakeError::WouldBlock(s)) => {
                    res = Err(io::Error::from(io::ErrorKind::WouldBlock));
                    TConn::SSLC(s)
                }
                Err(_) => {
                    res = util::io_err("SSL Connection failed!");
                    TConn::Empty
                }
            },
            TConn::SSL(mut conn) => {
                res = conn.read(buf);
                TConn::SSL(conn)
            }
            _ => return util::io_err("Socket in failed state!"),
        };

        if let Ok(::std::usize::MAX) = res {
            debug!("SSL upgrade succeeded!");
            self.read(buf)
        } else {
            res
        }
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
            TConn::SSLC(conn) => match conn.handshake() {
                Ok(s) => {
                    res = Ok(::std::usize::MAX);
                    TConn::SSL(s)
                }
                Err(HandshakeError::WouldBlock(s)) => {
                    res = Err(io::Error::from(io::ErrorKind::WouldBlock));
                    TConn::SSLC(s)
                }
                Err(_) => return util::io_err("SSL Connection failed!"),
            },
            TConn::SSL(mut conn) => {
                res = conn.write(buf);
                TConn::SSL(conn)
            }
            _ => return util::io_err("Socket in failed state!"),
        };

        if let Ok(::std::usize::MAX) = res {
            debug!("SSL upgrade succeeded!");
            self.write(buf)
        } else {
            res
        }
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
