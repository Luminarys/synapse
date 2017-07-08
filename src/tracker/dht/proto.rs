use std::net::SocketAddr;
use super::{DHT_ID, ID, Distance, BUCKET_MAX};

pub struct Node {
    id: ID,
    addr: SocketAddr,
}

pub enum Request {
    Ping,
    FindNode(ID),
    GetPeers([u8; 20]),
    AnnouncePeer { hash: [u8; 20], token: Vec<u8> },
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

