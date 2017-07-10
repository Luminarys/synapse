use std::net::UdpSocket;
use std::{io, sync};
use num::bigint::BigUint;
use {amy, tracker, CONFIG};
use slog::Logger;

mod rt;
mod proto;

type ID = BigUint;
type Distance = BigUint;

lazy_static! {
    pub static ref DHT_ID: ID = {
        use rand::{self, Rng};

        let mut id = [0u8; 20];
        let mut rng = rand::thread_rng();
        for i in 0..20 {
            id[i] = rng.gen::<u8>();
        }
        BigUint::from_bytes_be(&id)
    };
}

const BUCKET_MAX: usize = 8;
const VERSION: &'static str = "SY";

pub struct Manager {
    id: usize,
    table: rt::RoutingTable,
    sock: UdpSocket,
    buf: Vec<u8>,
    l: Logger,
}

impl Manager {
    pub fn new(reg: &sync::Arc<amy::Registrar>, l: Logger) -> io::Result<Manager> {
        let sock = UdpSocket::bind(("0.0.0.0", CONFIG.dht_port))?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;

        let table = if let Some(t) = rt::RoutingTable::deserialize() {
            t
        } else {
            info!(l, "DHT table could not be read from disk, creating new table!");
            let mut t = rt::RoutingTable::new();
            if let Some(addr) = CONFIG.dht_bootstrap_node {
                info!(l, "Using bootstrap node!");
                let (msg, _) = t.add_addr(addr.clone());
                sock.send_to(&msg.encode(), addr).unwrap();
            }
            t
        };

        Ok(Manager {
            table,
            sock,
            id,
            buf: vec![0u8; 500],
            l
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn readable(&mut self) -> Vec<tracker::Response> {
        let mut resps = Vec::new();
        loop {
            match self.sock.recv_from(&mut self.buf[..]) {
                Ok((v, addr)) => {
                    if let Ok(req) = proto::Request::decode(&self.buf[..v]) {
                        let resp = self.table.handle_req(req).encode();
                        if self.sock.send_to(&resp, addr).is_err() {
                            warn!(self.l, "Failed to send message on UDP socket!");
                        }
                    } else if let Ok(resp) = proto::Response::decode(&self.buf[..v]) {
                        match self.table.handle_resp(resp) {
                            Ok(r) => resps.push(r),
                            Err(q) => {
                                for (req, a) in q {
                                    if self.sock.send_to(&req.encode(), a).is_err() {
                                        warn!(self.l, "Failed to send message on UDP socket!");
                                    }
                                }
                            }
                        }
                    } else {
                        debug!(self.l, "Received invalid message from {:?}!", addr);
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    } else {
                        warn!(self.l, "Encountered unexpected error reading from UDP socket: {:?}!", e);
                        break;
                    }
                }
            }
        }
        resps
    }

    pub fn tick(&mut self) {
        for (req, a) in self.table.tick() {
            if self.sock.send_to(&req.encode(), a).is_err() {
                warn!(self.l, "Failed to send message on UDP socket!");
            }
        }
    }
}
