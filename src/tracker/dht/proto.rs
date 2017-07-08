use std::net::SocketAddr;
use std::collections::BTreeMap;
use super::{DHT_ID, ID, Distance, BUCKET_MAX};
use bencode::{self, BEncode};

const VERSION: &'static str = "SY";

error_chain! {
    errors {
        Generic(r: String) {
            description("generic error")
            display("generic node error: {}", r)
        }

        Server(r: String) {
            description("server error")
            display("server error: {}", r)
        }

        Protocol(r: String) {
            description("protocol error")
            display("protocol error: {}", r)
        }

        MethodUnknown(r: String) {
            description("method unknown")
            display("method unknown: {}", r)
        }

        InvalidResponse(r: &'static str) {
            description("invalid response")
            display("invalid response: {}", r)
        }
    }
}

pub struct Node {
    pub id: ID,
    pub addr: SocketAddr,
}

pub struct Request {
    transaction: Vec<u8>,
    kind: RequestKind
}

pub enum RequestKind {
    Ping,
    FindNode(ID),
    GetPeers([u8; 20]),
    AnnouncePeer { hash: [u8; 20], token: Vec<u8> },
}

impl Request {
    fn encode(self) -> Vec<u8> {
        let mut b = BTreeMap::new();
        b.insert(String::from("t"), BEncode::String(self.transaction));
        b.insert(String::from("y"), BEncode::from_str("q"));
        b.insert(String::from("v"), BEncode::from_str(VERSION));
        match self.kind {
            Ping => {
                b.insert(String::from("q"), BEncode::from_str("ping"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(DHT_ID.to_bytes_be()));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            _ => { }
        }
        BEncode::Dict(b).encode_to_buf()
    }
}

pub enum Response {
    Ping(ID),
    FindNode(Vec<Node>),
    GetPeers { token: Vec<u8>, resp: PeerResp },
    AnnouncePeer(ID),
}

pub enum PeerResp {
    Values(Vec<SocketAddr>),
    Nodes(Vec<Node>),
}
