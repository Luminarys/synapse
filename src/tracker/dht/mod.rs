use std::net::UdpSocket;
use std::{io, sync};
use num::bigint::BigUint;
use tracker;
use {amy, CONFIG};

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
}

impl Manager {
    pub fn new(reg: &sync::Arc<amy::Registrar>) -> io::Result<Manager> {
        let sock = UdpSocket::bind(("0.0.0.0", CONFIG.dht_port))?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;

        Ok(Manager {
            table: rt::RoutingTable::new(),
            sock,
            id
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn readable(&mut self) -> Vec<tracker::Response> {
        let mut resps = Vec::new();
        resps
    }

    pub fn tick(&mut self) {
    }
}
