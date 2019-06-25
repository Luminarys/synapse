extern crate rand;
extern crate serde_json as json;
extern crate sha1;
extern crate ws;
extern crate synapse_rpc as rpc;
extern crate synapse_bencode as bencode;
extern crate quantiles;

use rand::prelude::*;
use bencode::BEncode;
use sha1::Sha1;
use quantiles::ckms::CKMS;
use std::collections::BTreeMap;
use std::time::Instant;

use rpc::message::{CMessage, SMessage};

const PIECE_LEN: usize = 16384;

struct Torrent {
    name: String,
    data: Vec<u8>,
    info: Vec<u8>,
}

// Returns (Torrent Info File, Torrent Data)
fn generate_torrent(size: usize) -> Torrent {
    let mut rng = rand::thread_rng();
    let mut data = Vec::with_capacity(size);
    for _ in 0..size {
        data.push(rng.gen());
    }
    let mut name = "SYSTAT".to_owned();
    for _ in 0..5 {
        name.push(rng.gen());
    }

    let mut top = BTreeMap::new();
    let mut info = BTreeMap::new();
    info.insert("name".to_owned(), BEncode::String(name.clone().into_bytes()));
    info.insert("piece length".to_owned(), BEncode::Int(PIECE_LEN as i64));
    let mut pieces = vec![];
    for c in data.chunks(PIECE_LEN) {
        pieces.extend(&Sha1::from(c).digest().bytes());
    }
    info.insert("pieces".to_owned(), BEncode::String(pieces));
    info.insert("length".to_owned(), BEncode::Int(size as i64));

    top.insert("announce".to_owned(), BEncode::String(vec![]));
    top.insert("info".to_owned(), BEncode::Dict(info));
    Torrent {
        name,
        data,
        info: BEncode::Dict(top).encode_to_buf(),
    }
}

enum TestKind {
    Init,
    AllTorrents,
}

struct Test {
    kind: TestKind,
    latency: CKMS<f64>,
    start: Instant,
    remaining: usize,
}

struct Dispatcher {
    sender: ws::Sender,
}

impl ws::Handler for Dispatcher {
    fn on_message(&mut self, msg: ws::Message) -> ws::Result<()> {
        match msg {
            ws::Message::Text(s) => {
                let data: SMessage = json::from_str(&s).map_err(Box::new)?;
                Ok(())
            }
            _ => Ok(())
        }
    }
}

fn main() -> ws::Result<()> {
    println!("Hello, world!");
    ws::connect("ws://127.0.0.1:8412", |sender| {
        Dispatcher { sender }
    })
}
