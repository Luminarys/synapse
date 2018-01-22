use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::os::unix::io::{AsRawFd, RawFd};

use {amy, c_ares};

use tracker::{ErrorKind, Result, ResultExt};
use util::FHashMap;

#[derive(Debug)]
pub struct QueryResponse {
    pub id: usize,
    pub res: Result<IpAddr>,
}

pub struct Resolver {
    reg: amy::Registrar,
    socks: FHashMap<usize, c_ares::Socket>,
    chan: c_ares::Channel,
    sender: Arc<Mutex<amy::Sender<QueryResponse>>>,
}

struct CSockWrapper(c_ares::Socket);

impl AsRawFd for CSockWrapper {
    fn as_raw_fd(&self) -> RawFd {
        self.0
    }
}

impl Resolver {
    pub fn new(reg: amy::Registrar, send: amy::Sender<QueryResponse>) -> Resolver {
        let mut opts = c_ares::Options::new();
        opts.set_timeout(3000).set_tries(4);
        Resolver {
            reg,
            socks: FHashMap::default(),
            chan: c_ares::Channel::with_options(opts).unwrap(),
            sender: Arc::new(Mutex::new(send)),
        }
    }

    pub fn contains(&self, id: usize) -> bool {
        self.socks.contains_key(&id)
    }

    pub fn readable(&mut self, id: usize) {
        self.chan.process_fd(self.socks[&id], c_ares::SOCKET_BAD);
    }

    pub fn writable(&mut self, id: usize) {
        self.chan.process_fd(c_ares::SOCKET_BAD, self.socks[&id]);
    }

    pub fn tick(&mut self) {
        // Add any unrecognized fd's to amy for polling, remove
        // any fd's in amy which are no longer handled by c_ares
        self.socks = self.chan
            .get_sock()
            .iter()
            .map(|(fd, _, _)| {
                (
                    self.reg
                        .register(&CSockWrapper(fd), amy::Event::Both)
                        .unwrap(),
                    fd,
                )
            })
            .collect();
    }

    pub fn new_query(&mut self, id: usize, host: &str) {
        // TODO: handle ipv6 too
        let s = self.sender.clone();
        self.chan
            .get_host_by_name(host, c_ares::AddressFamily::INET, move |res| {
                let res = res.chain_err(|| ErrorKind::DNS)
                    .and_then(|ips| ips.addresses().next().ok_or_else(|| ErrorKind::DNS.into()));
                let resp = QueryResponse { id, res };
                if s.lock().unwrap().send(resp).is_err() {
                    // Other end was shutdown, ignore
                }
            });
    }
}
