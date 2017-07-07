use std::collections::{HashSet, HashMap};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use tracker::{Result, ResultExt, ErrorKind};
use std::os::unix::io::{AsRawFd, RawFd};
use {amy, c_ares};

#[derive(Debug)]
pub struct QueryResponse {
    pub id: usize,
    pub res: Result<IpAddr>,
}

pub struct Resolver {
    reg: Arc<amy::Registrar>,
    socks: HashMap<usize, c_ares::Socket>,
    csocks: HashMap<c_ares::Socket, usize>,
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
    pub fn new(reg: Arc<amy::Registrar>, send: amy::Sender<QueryResponse>) -> Resolver {
        let mut opts = c_ares::Options::new();
        opts.set_timeout(3000)
            .set_tries(4);
        Resolver {
            reg,
            socks: HashMap::new(),
            csocks: HashMap::new(),
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
        self.chan.process_fd(self.socks[&id], c_ares::SOCKET_BAD);
    }

    pub fn tick(&mut self) {
        let mut marked = HashSet::new();
        // Add any unrecognized fd's to amy for polling, remove
        // any fd's in amy which are no longer handled by c_ares
        for (fd, _, _) in &self.chan.get_sock() {
            // Not efficient, but the max number of queries active at at time is likely limited
            if let Some(id) = self.csocks.get(&fd).cloned() {
                marked.insert(id);
            } else {
                let id = self.reg.register(&CSockWrapper(fd), amy::Event::Both).unwrap();
                marked.insert(id);
                self.socks.insert(id, fd);
                self.csocks.insert(fd, id);
            }
        }

        // Any socks remaining in csocks should be deregistered and destroyed
        Resolver::remove_socks(marked, &mut self.socks, &mut self.csocks, &self.reg);
    }

    fn remove_socks(
        marked: HashSet<usize>,
        socks: &mut HashMap<usize, c_ares::Socket>,
        csocks: &mut HashMap<c_ares::Socket, usize>,
        reg: &Arc<amy::Registrar>)
    {
        socks.retain(|id, fd| {
            if marked.contains(id) {
                true
            } else {
                csocks.remove(fd);
                if reg.deregister(&CSockWrapper(*fd)).is_err() {
                    // Probably from EINTR, so we're shutting down
                    // and it doesn't matter if it's ignored
                }
                false
            }
        });
    }

    pub fn new_query(&mut self, id: usize, host: &str) {
        // TODO: handle ipv6 too
        let s = self.sender.clone();
        self.chan.query_a(host, move |res| {
            let res = res.chain_err(|| ErrorKind::DNS).and_then(|ips| {
                ips.iter().next().ok_or(ErrorKind::DNS.into()).map(|ip| IpAddr::V4(ip.ipv4()))
            });
            let resp = QueryResponse {
                id,
                res,
            };
            if s.lock().unwrap().send(resp).is_err() {
                // Other end was shutdown, ignore
            }
        });
    }
}
