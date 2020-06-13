use std::io;
use std::net::{IpAddr, UdpSocket};

use crate::tracker::{ErrorKind, Result};

#[derive(Debug)]
pub struct QueryResponse {
    pub id: usize,
    pub res: Result<IpAddr>,
}

pub struct Resolver {
    pub id: usize,
    pub res: adns::Resolver,
    pub sock: UdpSocket,
}

impl Resolver {
    pub fn new(reg: &amy::Registrar) -> io::Result<Resolver> {
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;

        Ok(Resolver {
            id,
            sock,
            res: adns::Resolver::from_resolv()?,
        })
    }

    pub fn new_query(&mut self, id: usize, host: &str) -> io::Result<Option<IpAddr>> {
        self.res.query(&mut self.sock, id, host)
    }
}

impl From<adns::Response> for QueryResponse {
    fn from(resp: adns::Response) -> Self {
        QueryResponse {
            id: resp.id,
            res: match resp.result {
                Ok(ip) => Ok(ip),
                Err(adns::Error::NotFound) => Err(ErrorKind::DNSInvalid.into()),
                Err(adns::Error::Timeout) => Err(ErrorKind::DNSTimeout.into()),
            },
        }
    }
}
