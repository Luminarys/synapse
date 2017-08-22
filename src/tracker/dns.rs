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
    reg: amy::Registrar,
    socks: HashMap<usize, c_ares::Socket>,
    csocks: HashMap<c_ares::Socket, usize>,
    chan: c_ares::Channel,
    sender: Arc<Mutex<amy::Sender<QueryResponse>>>,
    marked: HashSet<usize>,
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
            socks: HashMap::new(),
            csocks: HashMap::new(),
            chan: c_ares::Channel::with_options(opts).unwrap(),
            sender: Arc::new(Mutex::new(send)),
            marked: HashSet::new(),
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
        self.marked.clear();
        // Add any unrecognized fd's to amy for polling, remove
        // any fd's in amy which are no longer handled by c_ares
        for (fd, _, _) in &self.chan.get_sock() {
            // Not efficient, but the max number of queries active at at time is likely limited
            if let Some(id) = self.csocks.get(&fd).cloned() {
                self.marked.insert(id);
            } else {
                let id = self.reg
                    .register(&CSockWrapper(fd), amy::Event::Both)
                    .unwrap();
                self.marked.insert(id);
                self.socks.insert(id, fd);
                self.csocks.insert(fd, id);
            }
        }

        // Any socks remaining in csocks should be deregistered and destroyed
        let socks = &mut self.socks;
        let csocks = &mut self.csocks;
        let marked = &self.marked;
        let reg = &self.reg;
        socks.retain(|id, fd| if marked.contains(id) {
            true
        } else {
            csocks.remove(fd);
            reg.deregister(&CSockWrapper(*fd)).ok();
            false
        });
    }

    pub fn new_query(&mut self, id: usize, host: &str) {
        // TODO: handle ipv6 too
        let s = self.sender.clone();
        self.chan.get_host_by_name(
            host,
            c_ares::AddressFamily::INET,
            move |res| {
                let res = res.chain_err(|| ErrorKind::DNS).and_then(|ips| {
                    ips.addresses().next().ok_or_else(|| ErrorKind::DNS.into())
                });
                let resp = QueryResponse { id, res };
                if s.lock().unwrap().send(resp).is_err() {
                    // Other end was shutdown, ignore
                }
            },
        );
    }
}
