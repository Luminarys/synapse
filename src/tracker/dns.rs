use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use {amy, c_ares};

use tracker::{ErrorKind, Result, ResultExt};

#[derive(Debug)]
pub struct QueryResponse {
    pub id: usize,
    pub res: Result<IpAddr>,
}

pub struct Resolver {
    chan: c_ares::Channel,
    sender: Arc<Mutex<amy::Sender<QueryResponse>>>,
}

impl Resolver {
    pub fn new(send: amy::Sender<QueryResponse>) -> Resolver {
        let mut opts = c_ares::Options::new();
        opts.set_timeout(3000).set_tries(4);
        Resolver {
            chan: c_ares::Channel::with_options(opts).unwrap(),
            sender: Arc::new(Mutex::new(send)),
        }
    }

    pub fn tick(&mut self) {
        let mut rfd = vec![];
        let mut wfd = vec![];
        // Not efficient, but the max number of queries active at at time is likely limited
        for (fd, r, w) in &self.chan.get_sock() {
            if r {
                rfd.push(fd);
            }
            if w {
                wfd.push(fd);
            }
        }

        for fd in rfd {
            self.chan.process_fd(fd, c_ares::SOCKET_BAD);
        }

        for fd in wfd {
            self.chan.process_fd(c_ares::SOCKET_BAD, fd);
        }
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
