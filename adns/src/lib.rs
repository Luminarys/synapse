extern crate dns_parser;
extern crate resolv_conf;

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

const QUERY_TIMEOUT_MS: u64 = 1000;

pub struct Resolver {
    servers: Vec<SocketAddr>,
    cache: HashMap<String, CacheEntry>,
    queries: HashMap<u16, Query>,
    responses: HashMap<String, Vec<usize>>,
    buf: Vec<u8>,
    qnum: u16,
    timeout: Duration,
}

struct Query {
    domain: String,
    query_deadline: Instant,
    deadline: Instant,
    v4: bool,
    server: usize,
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
    pub fn new(servers: &[SocketAddr]) -> Resolver {
        let buf = vec![0u8; 512];
        Resolver {
            servers: servers.to_owned(),
            queries: HashMap::new(),
            responses: HashMap::new(),
            cache: HashMap::new(),
            timeout: Duration::from_secs(3),
            buf,
            qnum: 0,
        }
    }

    pub fn purge(&mut self) {
        self.cache.clear();
    }

    pub fn from_resolv() -> io::Result<Resolver> {
        let buf = vec![0u8; 512];
        let mut conf = Vec::with_capacity(4096);
        let mut f = File::open("/etc/resolv.conf")?;
        f.read_to_end(&mut conf)?;
        let cfg = resolv_conf::Config::parse(&conf).map_err(|e| {
            io::Error::new(io::ErrorKind::Other, format!("invalid resolv.conf: {}", e))
        })?;

        let servers: Vec<_> = cfg
            .nameservers
            .into_iter()
            .filter_map(|ip| match ip {
                resolv_conf::ScopedIp::V4(ip) => Some(SocketAddr::new(IpAddr::V4(ip), 53)),
                resolv_conf::ScopedIp::V6(ip, _) => Some(SocketAddr::new(IpAddr::V6(ip), 53)),
            })
            .collect();

        if servers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "No nameservers found in resolv.conf!",
            ));
        }

        Ok(Resolver {
            servers,
            queries: HashMap::new(),
            responses: HashMap::new(),
            cache: HashMap::new(),
            timeout: Duration::from_secs(cfg.timeout as u64),
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
        if self.servers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "No nameservers provided",
            ));
        }

        if let Some(entry) = self.cache.get(domain) {
            return Ok(Some(entry.ip));
        }
        if let Ok(entry) = domain.parse() {
            return Ok(Some(entry));
        }
        if self.responses.get(domain).is_none() {
            let qn = self.qnum;
            self.qnum = self.qnum.wrapping_add(1);
            let mut query = dns_parser::Builder::new_query(qn, true);
            query.add_question(domain, dns_parser::QueryType::A, dns_parser::QueryClass::IN);
            let packet = query.build().unwrap_or_else(|d| d);
            sock.send_to(&packet, self.servers[0])?;

            self.responses.insert(domain.to_string(), vec![]);
            let now = Instant::now();
            self.queries.insert(
                qn,
                Query {
                    v4: true,
                    server: 0,
                    domain: domain.to_string(),
                    deadline: now + self.timeout,
                    query_deadline: now + Duration::from_millis(QUERY_TIMEOUT_MS),
                },
            );
        }
        self.responses.get_mut(domain).unwrap().push(id);
        Ok(None)
    }

    pub fn read<F: FnMut(Response)>(&mut self, sock: &mut UdpSocket, mut f: F) -> io::Result<()> {
        'process: loop {
            match sock.recv_from(&mut self.buf) {
                Ok((amnt, _)) => {
                    match dns_parser::Packet::parse(&self.buf[..amnt]) {
                        Ok(packet) => {
                            let qn = packet.header.id;
                            let mut q = match self.queries.remove(&qn) {
                                Some(q) => q,
                                // This could happen if timeout is exceeeded but we eventually get
                                // a response, ignore.
                                None => continue,
                            };
                            let now = Instant::now();
                            for answer in packet.answers {
                                match answer.data {
                                    dns_parser::RRData::A(addr) => {
                                        for id in self.responses.remove(&q.domain).unwrap() {
                                            f(Response {
                                                id,
                                                result: Ok(addr.into()),
                                            });
                                        }
                                        self.cache.insert(
                                            q.domain.to_owned(),
                                            CacheEntry {
                                                ip: addr.into(),
                                                deadline: now
                                                    + Duration::from_secs(answer.ttl.into()),
                                            },
                                        );
                                        continue 'process;
                                    }
                                    dns_parser::RRData::AAAA(addr) => {
                                        for id in self.responses.remove(&q.domain).unwrap() {
                                            f(Response {
                                                id,
                                                result: Ok(addr.into()),
                                            });
                                        }
                                        self.cache.insert(
                                            q.domain.to_owned(),
                                            CacheEntry {
                                                ip: addr.into(),
                                                deadline: now
                                                    + Duration::from_secs(answer.ttl.into()),
                                            },
                                        );
                                        continue 'process;
                                    }
                                    _ => continue,
                                }
                            }
                            let pkt = q.next(qn);
                            if q.server != self.servers.len() {
                                sock.send_to(&pkt, self.servers[q.server])?;
                                self.queries.insert(qn, q);
                            } else {
                                for id in self.responses.remove(&q.domain).unwrap() {
                                    f(Response {
                                        id,
                                        result: Err(Error::NotFound),
                                    });
                                }
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

    pub fn tick<F: FnMut(Response)>(&mut self, sock: &mut UdpSocket, mut f: F) -> io::Result<()> {
        let now = Instant::now();
        let responses = &mut self.responses;
        let servers = &self.servers;
        let mut res = Ok(());
        self.cache.retain(|_, entry| now < entry.deadline);
        self.queries.retain(|qn, query| {
            if now > query.query_deadline {
                if now > query.deadline {
                    for id in responses.remove(&query.domain).unwrap() {
                        f(Response {
                            id,
                            result: Err(Error::Timeout),
                        });
                    }
                } else {
                    let pkt = query.next(*qn);
                    if query.server != servers.len() {
                        res = sock.send_to(&pkt, servers[query.server]).map(|_| ());
                        return true;
                    } else {
                        for id in responses.remove(&query.domain).unwrap() {
                            f(Response {
                                id,
                                result: Err(Error::Timeout),
                            });
                        }
                    }
                }
                false
            } else {
                true
            }
        });
        res
    }
}

impl Query {
    pub fn next(&mut self, qn: u16) -> Vec<u8> {
        self.query_deadline = Instant::now() + Duration::from_millis(QUERY_TIMEOUT_MS);
        if self.v4 {
            self.v4 = false;
            let mut query = dns_parser::Builder::new_query(qn, true);
            query.add_question(
                &self.domain,
                dns_parser::QueryType::AAAA,
                dns_parser::QueryClass::IN,
            );
            query.build().unwrap_or_else(|d| d)
        } else {
            self.server += 1;
            self.v4 = true;
            let mut query = dns_parser::Builder::new_query(qn, true);
            query.add_question(
                &self.domain,
                dns_parser::QueryType::A,
                dns_parser::QueryClass::IN,
            );
            query.build().unwrap_or_else(|d| d)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_google() {
        let mut resolver = Resolver::new(&["8.8.8.8:53".parse().unwrap()]);
        let mut sock = UdpSocket::bind("0.0.0.0:0").unwrap();
        sock.set_nonblocking(true).unwrap();

        assert_eq!(resolver.query(&mut sock, 0, "google.com").unwrap(), None);
        assert_eq!(resolver.query(&mut sock, 1, "google.com").unwrap(), None);
        assert_eq!(resolver.responses.get("google.com").unwrap().len(), 2);
        std::thread::sleep(Duration::from_millis(100));
        resolver
            .tick(&mut sock, |_| {
                panic!("timeout should not have occured yet!")
            })
            .unwrap();
        let mut count = 0;
        resolver
            .read(&mut sock, |resp| {
                count += 1;
                assert!(resp.result.is_ok());
            })
            .unwrap();
        assert_eq!(count, 2);

        assert!(resolver
            .query(&mut sock, 0, "google.com")
            .unwrap()
            .is_some());

        resolver
            .query(&mut sock, 0, "thiswebsiteshouldexit12589t69.com")
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        resolver
            .read(&mut sock, |_| panic!("AAAA resolution should be attmpted"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let mut processed = false;
        resolver
            .read(&mut sock, |resp| {
                processed = true;
                assert_eq!(resp.result, Err(Error::NotFound))
            })
            .unwrap();
        #[cfg(not(target_os = "macos"))]
        assert!(processed);
    }
}
