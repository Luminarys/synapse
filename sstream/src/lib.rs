mod no_verify_tls;

use std::io::{self, Read};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;

use net2::{TcpBuilder, TcpStreamExt};
use rustls::{self, Session};
use webpki;
use webpki_roots;
use crate::no_verify_tls::NoVerifyTLS;

const EINPROGRESS: i32 = 115;

/// Nonblocking Secure TcpStream implementation.
pub struct SStream {
    conn: SConn,
    fd: i32,
}

enum SConn {
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

#[derive(Copy, Clone)]
pub struct SStreamConfig {
    tls_check_certificates: bool,
}

impl SStreamConfig {
    pub fn with_tls_check_certificates(
        &self,
        tls_no_verify: bool,
    ) -> SStreamConfig {
        SStreamConfig {
            tls_check_certificates: tls_no_verify,

            ..self.clone()
        }
    }
}

impl Default for SStreamConfig {
    fn default() -> Self {
        SStreamConfig {
            tls_check_certificates: true,
        }
    }
}

impl SStream {
    pub fn new_v6(host: Option<String>, config: Option<SStreamConfig>) -> io::Result<SStream> {
        let conn = TcpBuilder::new_v6()?.to_tcp_stream()?;
        SStream::new(conn, host, config)
    }

    pub fn new_v4(host: Option<String>, config: Option<SStreamConfig>) -> io::Result<SStream> {
        let conn = TcpBuilder::new_v4()?.to_tcp_stream()?;
        SStream::new(conn, host, config)
    }

    fn new(conn: TcpStream, host: Option<String>, config: Option<SStreamConfig>) -> io::Result<SStream> {
        conn.set_nonblocking(true)?;
        let fd = conn.as_raw_fd();
        let sock = match host {
            Some(h) => {
                let tls_check_certificates =
                    if let Some(config) = config {
                        config.tls_check_certificates
                    } else {
                        true
                    };

                let mut tls_config = rustls::ClientConfig::new();

                tls_config
                    .root_store
                    .add_server_trust_anchors(&webpki_roots::TLS_SERVER_ROOTS);

                if !tls_check_certificates {
                    tls_config.client_auth_cert_resolver = Arc::new(NoVerifyTLS);
                }

                let dns_name = match webpki::DNSNameRef::try_from_ascii_str(&h) {
                    Ok(name) => name,
                    Err(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "invalid host string used",
                        ))
                    }
                };

                let session = rustls::ClientSession::new(
                    &Arc::new(tls_config),
                    dns_name,
                );

                SStream {
                    conn: SConn::SSLC { conn, session },
                    fd,
                }
            }
            None => SStream {
                conn: SConn::Plain(conn),
                fd,
            },
        };
        Ok(sock)
    }

    pub fn connect(&mut self, addr: SocketAddr) -> io::Result<()> {
        match self.conn {
            SConn::Plain(ref mut c)
            | SConn::SSLC {
                conn: ref mut c, ..
            } => {
                if let Err(e) = c.connect(addr) {
                    if Some(EINPROGRESS) != e.raw_os_error() {
                        return Err(e);
                    }
                }
                Ok(())
            }
            SConn::SSLS { .. } => unreachable!("Server side TLS connect"),
        }
    }

    pub fn from_plain(stream: TcpStream) -> io::Result<SStream> {
        stream.set_nonblocking(true)?;
        let fd = stream.as_raw_fd();
        Ok(SStream {
            conn: SConn::Plain(stream),
            fd,
        })
    }

    pub fn from_ssl(conn: TcpStream, config: &Arc<rustls::ServerConfig>) -> io::Result<SStream> {
        conn.set_nonblocking(true)?;
        let fd = conn.as_raw_fd();
        let session = rustls::ServerSession::new(config);
        Ok(SStream {
            conn: SConn::SSLS { conn, session },
            fd,
        })
    }

    pub fn get_stream(&self) -> &TcpStream {
        match self.conn {
            SConn::Plain(ref c) => c,
            SConn::SSLC { ref conn, .. } => conn,
            SConn::SSLS { ref conn, .. } => conn,
        }
    }

    fn read_(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.conn {
            SConn::Plain(ref mut c) => c.read(buf),
            SConn::SSLC {
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
            SConn::SSLS {
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

impl io::Read for SStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.read_(buf) {
            Ok(n) => Ok(n),
            Err(e) => {
                if e.kind() == io::ErrorKind::ConnectionAborted {
                    return Ok(0);
                }
                return Err(e);
            }
        }
    }
}

impl io::Write for SStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.conn {
            SConn::Plain(ref mut c) => c.write(buf),
            SConn::SSLC {
                ref mut conn,
                ref mut session,
            } => {
                let result = session.write(buf);
                session.complete_io(conn)?;
                result
            }
            SConn::SSLS {
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
            SConn::Plain(ref mut c) => c.flush(),
            SConn::SSLC {
                ref mut conn,
                ref mut session,
            } => {
                session.flush()?;
                conn.flush()
            }
            SConn::SSLS {
                ref mut conn,
                ref mut session,
            } => {
                session.flush()?;
                conn.flush()
            }
        }
    }
}

impl AsRawFd for SStream {
    fn as_raw_fd(&self) -> RawFd {
        self.fd
    }
}

// TODO: Add tests
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
