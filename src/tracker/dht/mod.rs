use std::net::{SocketAddr, UdpSocket};
use std::io;
use self::io::{Read, Write};
use num::bigint::BigUint;
use {amy, tracker, CONFIG};
use std::{thread, time};
use slog::Logger;
use std::fs::OpenOptions;
use std::path::Path;

mod rt;
mod proto;

type ID = BigUint;

const BUCKET_MAX: usize = 8;
const VERSION: &'static str = "SY";
const SESSION_FILE: &'static str = "dht_data";
const MIN_BOOTSTRAP_BKTS: usize = 3;
const TX_TIMEOUT_SECS: i64 = 20;

pub struct Manager {
    id: usize,
    table: rt::RoutingTable,
    dht_flush: time::Instant,
    sock: UdpSocket,
    buf: Vec<u8>,
    l: Logger,
}

impl Manager {
    pub fn new(reg: &amy::Registrar, l: Logger) -> io::Result<Manager> {
        let sock = UdpSocket::bind(("0.0.0.0", CONFIG.dht_port))?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;

        let p = Path::new(&CONFIG.session[..]).join(SESSION_FILE);
        let mut data = Vec::new();
        if let Ok(mut f) = OpenOptions::new().read(true).open(&p) {
            f.read_to_end(&mut data)?;
        }
        let table = if let Some(t) = rt::RoutingTable::deserialize(&data[..]) {
            info!(l, "DHT table loaded from disk!");
            t
        } else {
            info!(
                l,
                "DHT table could not be read from disk, creating new table!"
            );
            let mut t = rt::RoutingTable::new();
            if let Some(addr) = CONFIG.dht_bootstrap_node {
                info!(l, "Using bootstrap node!");
                let (msg, _) = t.add_addr(addr.clone());
                sock.send_to(&msg.encode(), addr)?;
            }
            t
        };

        Ok(Manager {
            table,
            sock,
            id,
            buf: vec![0u8; 500],
            dht_flush: time::Instant::now(),
            l,
        })
    }

    pub fn init(&mut self) {
        debug!(self.l, "Initializing DHT nodes!");
        for (q, a) in self.table.init() {
            self.send_msg(&q.encode(), a);
        }
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn readable(&mut self) -> Vec<tracker::Response> {
        let mut resps = Vec::new();
        loop {
            match self.sock.recv_from(&mut self.buf[..]) {
                Ok((v, addr)) => {
                    trace!(self.l, "Processing msg from {:?}!", addr);
                    if let Ok(req) = proto::Request::decode(&self.buf[..v]) {
                        let resp = self.table.handle_req(req, addr).encode();
                        self.send_msg(&resp, addr);
                    } else if let Ok(resp) = proto::Response::decode(&self.buf[..v]) {
                        match self.table.handle_resp(resp, addr) {
                            Ok(r) => resps.push(r),
                            Err(q) => {
                                for (req, a) in q {
                                    self.send_msg(&req.encode(), a);
                                }
                            }
                        }
                    } else {
                        trace!(self.l, "Received invalid message from {:?}!", addr);
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    } else {
                        warn!(
                            self.l,
                            "Encountered unexpected error reading from UDP socket: {:?}!",
                            e
                        );
                        break;
                    }
                }
            }
        }
        resps
    }

    pub fn get_peers(&mut self, tid: usize, hash: [u8; 20]) {
        for (req, a) in self.table.get_peers(tid, hash) {
            self.send_msg(&req.encode(), a);
        }
    }

    pub fn add_addr(&mut self, addr: SocketAddr) {
        self.table.add_addr(addr);
    }

    pub fn announce(&mut self, hash: [u8; 20]) {
        for (req, a) in self.table.announce(hash) {
            self.send_msg(&req.encode(), a);
        }
    }

    pub fn tick(&mut self) {
        if self.dht_flush.elapsed() > time::Duration::from_secs(60) {
            let data = self.table.serialize();
            thread::spawn(move || {
                let p = Path::new(&CONFIG.session[..]).join(SESSION_FILE);
                if let Err(e) = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(&p)
                    .and_then(|mut f| f.write(&data[..]))
                {
                    // TODO: properly log
                    println!("DHT serialization failed: {:?}!", e);
                }
            });
            self.dht_flush = time::Instant::now();
        }
        for (req, a) in self.table.tick() {
            self.send_msg(&req.encode(), a);
        }
    }

    fn send_msg(&mut self, msg: &[u8], addr: SocketAddr) {
        if let Err(e) = self.sock.send_to(msg, addr) {
            warn!(self.l, "Failed to send message on UDP socket: {:?}", e);
        }
    }
}
