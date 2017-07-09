use std::collections::BTreeMap;
use std::net::SocketAddr;
use super::{DHT_ID, ID};
use bencode::{self, BEncode};
use num::bigint::BigUint;
use util::{addr_to_bytes, bytes_to_addr};
use CONFIG;

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

pub struct Response {
    transaction: Vec<u8>,
    kind: ResponseKind,
}

pub enum ResponseKind {
    ID(ID),
    FindNode { id: ID, nodes: Vec<Node> },
    GetPeers { id: ID, token: Vec<u8>, resp: PeerResp },
    Error(ErrorKind)
}

pub enum PeerResp {
    Values(Vec<SocketAddr>),
    Nodes(Vec<Node>),
}

impl Request {
    fn encode(self) -> Vec<u8> {
        let mut b = BTreeMap::new();
        b.insert(String::from("t"), BEncode::String(self.transaction));
        b.insert(String::from("y"), BEncode::from_str("q"));
        b.insert(String::from("v"), BEncode::from_str(VERSION));
        match self.kind {
            RequestKind::Ping => {
                b.insert(String::from("q"), BEncode::from_str("ping"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(DHT_ID.to_bytes_be()));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::FindNode(id) => {
                b.insert(String::from("q"), BEncode::from_str("find_node"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(DHT_ID.to_bytes_be()));
                args.insert(String::from("target"), BEncode::String(id.to_bytes_be()));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::GetPeers(hash) => {
                b.insert(String::from("q"), BEncode::from_str("find_node"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(DHT_ID.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(String::from("info_hash"), BEncode::String(ib));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::AnnouncePeer { hash, token } => {
                b.insert(String::from("q"), BEncode::from_str("announce_peer"));
                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(DHT_ID.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(String::from("info_hash"), BEncode::String(ib));
                // TODO: Consider changing this once uTP is implemented
                args.insert(String::from("implied_port"), BEncode::Int(0));
                args.insert(String::from("port"), BEncode::Int(CONFIG.port as i64));
                args.insert(String::from("token"), BEncode::String(token));

                b.insert(String::from("a"), BEncode::Dict(args));

            }
        }
        BEncode::Dict(b).encode_to_buf()
    }
}

impl Response {
    fn encode(self) -> Vec<u8> {
        let mut b = BTreeMap::new();
        let is_err = self.is_err();
        b.insert(String::from("t"), BEncode::String(self.transaction));
        let mut args = BTreeMap::new();
        match self.kind {
            ResponseKind::ID(id) => {
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
            }
            ResponseKind::FindNode { id, nodes } => {
                let mut data = Vec::new();
                for node in nodes {
                    data.extend(node.to_bytes())
                }
                args.insert(String::from("nodes"), BEncode::String(data));
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
            }
            ResponseKind::GetPeers { id, token, resp } => {
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
                args.insert(String::from("token"), BEncode::String(token));
                match resp {
                    PeerResp::Values(addrs) => {
                        let mut data = Vec::new();
                        for addr in addrs {
                            data.extend_from_slice(&addr_to_bytes(&addr)[..]);
                        }
                        args.insert(String::from("values"), BEncode::String(data));
                    }
                    PeerResp::Nodes(nodes) => {
                        let mut data = Vec::new();
                        for node in nodes {
                            data.extend(node.to_bytes())
                        }
                        args.insert(String::from("nodes"), BEncode::String(data));
                    }
                }
            }
            ResponseKind::Error(e) => {
                let mut err = Vec::new();
                match e {
                    ErrorKind::Generic(msg) => {
                        err.push(BEncode::from_int(201));
                        err.push(BEncode::from_str(&msg));
                    }
                    ErrorKind::Server(msg) => {
                        err.push(BEncode::from_int(202));
                        err.push(BEncode::from_str(&msg));
                    }
                    ErrorKind::Protocol(msg) => {
                        err.push(BEncode::from_int(203));
                        err.push(BEncode::from_str(&msg));
                    }
                    ErrorKind::MethodUnknown(msg) => {
                        err.push(BEncode::from_int(204));
                        err.push(BEncode::from_str(&msg));
                    }
                    _ => unreachable!(),
                }
                b.insert(String::from("e"), BEncode::List(err));
            }
        }
        if is_err {
                b.insert(String::from("y"), BEncode::from_str("e"));
        } else {
                b.insert(String::from("y"), BEncode::from_str("r"));
                b.insert(String::from("r"), BEncode::Dict(args));
        }
        BEncode::Dict(b).encode_to_buf()
    }

    fn decode(buf: Vec<u8>) -> Result<Response> {
        let b: BEncode = bencode::decode_buf(&buf).chain_err(|| ErrorKind::InvalidResponse("Invalid BEncoded data"))?;
        let mut d = b.to_dict()
            .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(must be dict)").into())?;
        let transaction = d.remove("t")
            .and_then(|b| b.to_bytes())
            .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(dict must have t field)").into())?;
        let y = d.remove("y")
            .and_then(|b| b.to_string())
            .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(dict must have y field)").into())?;
        match &y[..] {
            "e" => {
                let mut e = d.remove("e")
                    .and_then(|b| b.to_list())
                    .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(error resp must have e field)").into())?;
                if e.len() != 2 {
                    return Err(ErrorKind::InvalidResponse("Invalid BEncoded data(e field must have two terms)").into());
                }
                let code = e.remove(0)
                    .to_int()
                    .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(e field must start with integer code)").into())?;
                let msg = e.remove(0)
                    .to_string()
                    .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(e field must end with string data)").into())?;
                let err = match code {
                    201 => ErrorKind::Generic(msg),
                    202 => ErrorKind::Server(msg),
                    203 => ErrorKind::Protocol(msg),
                    204 => ErrorKind::MethodUnknown(msg),
                    _ => return Err(ErrorKind::InvalidResponse("Invalid BEncoded data(invalid error code)").into()),
                };
                Ok(Response { transaction, kind: ResponseKind::Error(err)})
            }
            "r" => {
                let mut r = d.remove("r")
                    .and_then(|b| b.to_dict())
                    .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(resp must have r field)").into())?;

                let id = r.remove("id")
                    .and_then(|b| b.to_bytes())
                    .map(|b| BigUint::from_bytes_be(&b))
                    .ok_or::<Error>(ErrorKind::InvalidResponse("Invalid BEncoded data(response must have id)").into())?;

                let kind = if let Some(token) = r.remove("token").and_then(|b| b.to_bytes()) {
                    if let Some(data) = r.remove("values").and_then(|b| b.to_bytes()) {
                        let mut peers = Vec::new();
                        for d in data.chunks(6) {
                            peers.push(bytes_to_addr(d));
                        }
                        ResponseKind::GetPeers { id, token, resp: PeerResp::Values(peers) }
                    } else if let Some(ns) = r.remove("nodes").and_then(|b| b.to_bytes()) {
                        let mut nodes = Vec::new();
                        for n in ns.chunks(26) {
                            nodes.push(Node::new(n));
                        }
                        ResponseKind::GetPeers { id, token, resp: PeerResp::Nodes(nodes) }
                    } else {
                        return Err(ErrorKind::InvalidResponse("Invalid BEncoded data(get_peers resp has no values/nodes fields)").into());
                    }
                } else if let Some(ns) = r.remove("nodes").and_then(|b| b.to_bytes()) {
                    let mut nodes = Vec::new();
                    for n in ns.chunks(26) {
                        nodes.push(Node::new(n));
                    }
                    ResponseKind::FindNode { id, nodes }
                } else {
                    ResponseKind::ID(id)
                };
                Ok(Response { transaction, kind })
            }
            _ => {
                Err(ErrorKind::InvalidResponse("Invalid BEncoded data(y field must be e/r)").into())
            }
        }
    }

    fn is_err(&self) -> bool {
        match self.kind {
            ResponseKind::Error(_) => true,
            _ => false,
        }
    }
}

impl Node {
    pub fn new(data: &[u8]) -> Node {
        let id = BigUint::from_bytes_be(&data[0..20]);
        Node { id, addr: bytes_to_addr(&data[20..]) }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = self.id.to_bytes_be();
        data.extend_from_slice(&addr_to_bytes(&self.addr)[..]);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::{Response, ResponseKind};
    use num::bigint::BigUint;

    #[test]
    fn test_decode_id_resp() {
        // {"t":"aa", "y":"r", "r": {"id":"mnopqrstuvwxyz123456"}}
        let r = Vec::from(&b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re"[..]);
        let d = Response::decode(r).unwrap();
        assert_eq!(d.transaction, b"aa");
        match d.kind {
            ResponseKind::ID(id) => {
                assert_eq!(id, BigUint::from_bytes_be(b"mnopqrstuvwxyz123456"));
            }
            _ => panic!("Should decode to ID!"),
        }
    }

    #[test]
    fn test_encode_decode_resp() {
        // {"t":"aa", "y":"r", "r": {"id":"mnopqrstuvwxyz123456"}}
        let r = Vec::from(&b"d1:rd2:id20:mnopqrstuvwxyz123456e1:t2:aa1:y1:re"[..]);
        let d = Response::decode(r.clone()).unwrap();
        assert_eq!(d.encode(), r);
    }
}
