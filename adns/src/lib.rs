extern crate dns_parser;
extern crate resolv_conf;

use std::time::{Duration, Instant};
use std::io::{self, Read};
use std::fs::File;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::collections::HashMap;

pub struct Resolver {
    server: SocketAddr,
    cache: HashMap<String, CacheEntry>,
    queries: HashMap<u16, Query>,
    buf: Vec<u8>,
    qnum: u16,
}

struct Query {
    id: usize,
    resps: u8,
    domain: String,
    deadline: Instant,
}

struct CacheEntry {
    ip: IpAddr,
    deadline: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Response {
    pub id: usize,
    pub result: Result<IpAddr, Error>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Error {
    NotFound,
    Timeout,
}

impl Resolver {
    pub fn new(server: SocketAddr) -> Resolver {
        let mut buf = Vec::with_capacity(512);
        unsafe {
            buf.set_len(512);
        }
        Resolver {
            server,
            queries: HashMap::new(),
            cache: HashMap::new(),
            buf,
            qnum: 0,
        }
    }

    pub fn from_resolv() -> io::Result<Resolver> {
        let mut buf = Vec::with_capacity(512);
        unsafe {
            buf.set_len(512);
        }

        let mut conf = Vec::with_capacity(4096);
        let mut f = File::open("/etc/resolv.conf")?;
        f.read_to_end(&mut conf)?;
        let cfg = resolv_conf::Config::parse(&conf).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("invalid resolv.conf: {}", e))
        })?;
        let server = match cfg.nameservers.first().cloned() {
            Some(resolv_conf::ScopedIp::V4(ip)) => SocketAddr::new(IpAddr::V4(ip), 53),
            Some(resolv_conf::ScopedIp::V6(ip, _)) => SocketAddr::new(IpAddr::V6(ip), 53),
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "No nameservers found in resolv.conf!",
                ))
            }
        };

        Ok(Resolver {
            server,
            queries: HashMap::new(),
            cache: HashMap::new(),
            buf,
            qnum: 0,
        })
    }

    pub fn query(
        &mut self,
        sock: &mut UdpSocket,
        id: usize,
        domain: &str,
    ) -> io::Result<Option<IpAddr>> {
        if let Some(entry) = self.cache.get(domain) {
            return Ok(Some(entry.ip));
        }

        let qn = self.qnum;
        self.qnum.wrapping_add(1);

        let mut query = dns_parser::Builder::new_query(qn, true);
        query.add_question(domain, dns_parser::QueryType::A, dns_parser::QueryClass::IN);
        let packet = query.build().unwrap_or_else(|d| d);
        sock.send_to(&packet, self.server)?;

        let mut query = dns_parser::Builder::new_query(qn, true);
        query.add_question(
            domain,
            dns_parser::QueryType::AAAA,
            dns_parser::QueryClass::IN,
        );
        let packet = query.build().unwrap_or_else(|d| d);
        sock.send_to(&packet, self.server)?;

        self.queries.insert(
            qn,
            Query {
                id,
                resps: 0,
                domain: domain.to_string(),
                deadline: Instant::now() + Duration::from_secs(3),
            },
        );
        Ok(None)
    }

    pub fn read<F: FnMut(Response)>(&mut self, sock: &mut UdpSocket, mut f: F) -> io::Result<()> {
        'process: loop {
            match sock.recv_from(&mut self.buf) {
                Ok((amnt, _)) => {
                    match dns_parser::Packet::parse(&self.buf[..amnt]) {
                        Ok(packet) => {
                            let qn = packet.header.id;
                            let (id, domain, mut resps, deadline) = match self.queries.remove(&qn) {
                                Some(q) => (q.id, q.domain, q.resps, q.deadline),
                                // This could happen if timeout is exceeeded but we eventually get
                                // a response, ignore.
                                None => continue,
                            };
                            let now = Instant::now();
                            for answer in packet.answers {
                                match answer.data {
                                    dns_parser::RRData::A(addr) => {
                                        self.cache.insert(
                                            domain,
                                            CacheEntry {
                                                ip: addr.into(),
                                                deadline: now
                                                    + Duration::from_secs(answer.ttl.into()),
                                            },
                                        );
                                        f(Response {
                                            id,
                                            result: Ok(addr.into()),
                                        });
                                        continue 'process;
                                    }
                                    dns_parser::RRData::AAAA(addr) => {
                                        self.cache.insert(
                                            domain,
                                            CacheEntry {
                                                ip: addr.into(),
                                                deadline: now
                                                    + Duration::from_secs(answer.ttl.into()),
                                            },
                                        );
                                        f(Response {
                                            id,
                                            result: Ok(addr.into()),
                                        });
                                        continue 'process;
                                    }
                                    _ => continue,
                                }
                            }
                            if resps == 0 {
                                self.queries.insert(
                                    qn,
                                    Query {
                                        id,
                                        domain,
                                        resps,
                                        deadline,
                                    },
                                );
                            } else {
                                f(Response {
                                    id,
                                    result: Err(Error::NotFound),
                                });
                            }
                        }
                        Err(e) => {
                            return Err(io::Error::new(
                                io::ErrorKind::Other,
                                format!("malformed dns packet received: {}", e),
                            ));
                        }
                    }
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }

    pub fn tick<F: FnMut(Response)>(&mut self, mut f: F) {
        let now = Instant::now();
        self.queries.retain(|_, query| {
            if now > query.deadline {
                f(Response {
                    id: query.id,
                    result: Err(Error::Timeout),
                });
                false
            } else {
                true
            }
        });
        self.cache.retain(|_, entry| now > entry.deadline);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google() {
        let mut resolver = Resolver::new("8.8.8.8:53".parse().unwrap());
        let mut sock = UdpSocket::bind("0.0.0.0:0").unwrap();
        sock.set_nonblocking(true).unwrap();

        assert_eq!(resolver.query(&mut sock, 0, "google.com").unwrap(), None);
        std::thread::sleep(Duration::from_millis(100));
        resolver.tick(|_| panic!("timeout should not have occured yet!"));
        resolver
            .read(&mut sock, |resp| assert!(resp.result.is_ok()))
            .unwrap();

        assert!(
            resolver
                .query(&mut sock, 0, "google.com")
                .unwrap()
                .is_some()
        );

        resolver
            .query(&mut sock, 0, "thiswebsiteshouldexit12589t69.com")
            .unwrap();
        std::thread::sleep(Duration::from_millis(100));
        resolver
            .read(&mut sock, |resp| {
                assert_eq!(resp.result, Err(Error::NotFound))
            })
            .unwrap();
    }
}
