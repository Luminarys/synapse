use std::io::{self, ErrorKind};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;

use net2::{TcpBuilder, TcpStreamExt};
use rustls::{self, Session};
use webpki;
use webpki_roots;

use crate::throttle::Throttle;
use crate::util;

/// Wrapper type over Mio sockets, allowing for use of UDP/TCP, encryption,
/// rate limiting, etc.
pub struct Socket {
    conn: TcpStream,
    addr: SocketAddr,
    pub throttle: Option<Throttle>,
}

const EINPROGRESS: i32 = 115;

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
            if Some(EINPROGRESS) != e.raw_os_error()
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
    Plain(TcpStream),
    SSLC {
        conn: TcpStream,
        session: rustls::ClientSession,
    },
    SSLS {
        conn: TcpStream,
        session: rustls::ServerSession,
    },
}

impl TSocket {
    pub fn new_v4(host: Option<String>) -> io::Result<TSocket> {
        let conn = TcpBuilder::new_v4()?.to_tcp_stream()?;
        conn.set_nonblocking(true)?;
        let fd = conn.as_raw_fd();
        let sock = match host {
            Some(h) => {
                let mut config = rustls::ClientConfig::new();
                config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);
                let dns_name = match webpki::DNSNameRef::try_from_ascii_str(&h) {
                    Ok(name) => name,
                    Err(_) => return util::io_err("Invalid hostname used"),
                };
                debug!("Initiating SSL connection to: {}", h);
                let session = rustls::ClientSession::new(&Arc::new(config), dns_name);
                TSocket {
                    conn: TConn::SSLC { conn, session },
                    fd,
                }
            }
            None => TSocket {
                conn: TConn::Plain(conn),
                fd,
            },
        };
        Ok(sock)
    }

    pub fn connect(&mut self, addr: SocketAddr) -> io::Result<()> {
        info!("Connecting to: {}", addr);
        match self.conn {
            TConn::Plain(ref mut c)
            | TConn::SSLC {
                conn: ref mut c, ..
            } => {
                if let Err(e) = c.connect(addr) {
                    if Some(EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                Ok(())
            }
            TConn::SSLS { .. } => unreachable!("Server side TLS connect"),
        }
    }

    pub fn from_plain(stream: TcpStream) -> io::Result<TSocket> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(TSocket {
            conn: TConn::Plain(stream),
            fd,
        })
    }

    pub fn from_ssl(conn: TcpStream, config: &Arc<rustls::ServerConfig>) -> io::Result<TSocket> {
        conn.set_nonblocking(true)?;
        let fd = conn.as_raw_fd();
        let session = rustls::ServerSession::new(config);
        Ok(TSocket {
            conn: TConn::SSLS { conn, session },
            fd,
        })
    }
}

impl io::Read for TSocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.conn {
            TConn::Plain(ref mut c) => c.read(buf),
            TConn::SSLC {
                ref mut conn,
                ref mut session,
            } => {
                // Attempt to call complete_io as many times as necessary
                // to complete handshaking. Once handshaking is complete
                // session.read should begin returning results which we
                // can then use. complete_io returning 0, 0 indicates that
                // EOF has been reached, but we still need to read out
                // the remaining bytes, propagating EOF. Prior to this
                // reading 0 bytes simply indicates the TLS session buffer
                // has no data
                loop {
                    match session.complete_io(conn)? {
                        (0, 0) => return session.read(buf),
                        _ => {
                            let res = session.read(buf)?;
                            if res > 0 {
                                return Ok(res);
                            }
                        }
                    }
                }
            }
            TConn::SSLS {
                ref mut conn,
                ref mut session,
            } => loop {
                match session.complete_io(conn)? {
                    (0, 0) => return session.read(buf),
                    _ => {
                        let res = session.read(buf)?;
                        if res > 0 {
                            return Ok(res);
                        }
                    }
                }
            },
        }
    }
}

impl io::Write for TSocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.conn {
            TConn::Plain(ref mut c) => c.write(buf),
            TConn::SSLC {
                ref mut conn,
                ref mut session,
            } => {
                let result = session.write(buf);
                session.complete_io(conn)?;
                result
            }
            TConn::SSLS {
                ref mut conn,
                ref mut session,
            } => {
                let result = session.write(buf);
                session.complete_io(conn)?;
                result
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.conn {
            TConn::Plain(ref mut c) => c.flush(),
            TConn::SSLC {
                ref mut conn,
                ref mut session,
            } => {
                session.flush()?;
                conn.flush()
            }
            TConn::SSLS {
                ref mut conn,
                ref mut session,
            } => {
                session.flush()?;
                conn.flush()
            }
        }
    }
}

impl AsRawFd for TSocket {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}
