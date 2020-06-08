use super::{ID, VERSION};
use crate::bencode::{self, BEncode};
use crate::util::{addr_to_bytes, bytes_to_addr};
use crate::CONFIG;
use num_bigint::BigUint;
use std::collections::BTreeMap;
use std::net::SocketAddr;
// use std::u16;

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

        InvalidRequest(r: &'static str) {
            description("invalid request")
                display("invalid request: {}", r)
        }
    }
}

#[derive(Debug)]
pub struct Request {
    pub transaction: Vec<u8>,
    pub version: Option<String>,
    pub kind: RequestKind,
}

#[derive(Debug)]
pub enum RequestKind {
    Ping(ID),
    FindNode {
        id: ID,
        target: ID,
    },
    GetPeers {
        id: ID,
        hash: [u8; 20],
    },
    AnnouncePeer {
        id: ID,
        hash: [u8; 20],
        token: Vec<u8>,
        port: u16,
        implied_port: bool,
    },
}

#[derive(Debug)]
pub struct Response {
    pub transaction: Vec<u8>,
    pub kind: ResponseKind,
}

#[derive(Debug)]
pub enum ResponseKind {
    ID(ID),
    FindNode {
        id: ID,
        nodes: Vec<Node>,
    },
    GetPeers {
        id: ID,
        token: Vec<u8>,
        values: Vec<SocketAddr>,
        nodes: Vec<Node>,
    },
    Error(ErrorKind),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    pub id: ID,
    pub addr: SocketAddr,
}

impl Request {
    pub fn ping(transaction: Vec<u8>, id: ID) -> Self {
        Request {
            transaction,
            version: Some(VERSION.to_owned()),
            kind: RequestKind::Ping(id),
        }
    }

    pub fn find_node(transaction: Vec<u8>, id: ID, target: ID) -> Self {
        Request {
            transaction,
            version: Some(VERSION.to_owned()),
            kind: RequestKind::FindNode { id, target },
        }
    }

    pub fn get_peers(transaction: Vec<u8>, id: ID, hash: [u8; 20]) -> Self {
        Request {
            transaction,
            version: Some(VERSION.to_owned()),
            kind: RequestKind::GetPeers { id, hash },
        }
    }

    pub fn announce(transaction: Vec<u8>, id: ID, hash: [u8; 20], token: Vec<u8>) -> Self {
        Request {
            transaction,
            version: Some(VERSION.to_owned()),
            kind: RequestKind::AnnouncePeer {
                id,
                hash,
                token,
                port: CONFIG.dht.port,
                implied_port: false,
            },
        }
    }

    pub fn encode(self) -> Vec<u8> {
        let mut b = BTreeMap::new();
        b.insert(String::from("t"), BEncode::String(self.transaction));
        b.insert(String::from("y"), BEncode::from_str("q"));
        if let Some(v) = self.version {
            b.insert(String::from("v"), BEncode::from_str(&v));
        }
        match self.kind {
            RequestKind::Ping(id) => {
                b.insert(String::from("q"), BEncode::from_str("ping"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::FindNode { id, target } => {
                b.insert(String::from("q"), BEncode::from_str("find_node"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
                args.insert(
                    String::from("target"),
                    BEncode::String(target.to_bytes_be()),
                );

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::GetPeers { id, hash } => {
                b.insert(String::from("q"), BEncode::from_str("get_peers"));

                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(String::from("info_hash"), BEncode::String(ib));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
            RequestKind::AnnouncePeer {
                id,
                hash,
                token,
                port,
                implied_port,
            } => {
                b.insert(String::from("q"), BEncode::from_str("announce_peer"));
                let mut args = BTreeMap::new();
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(String::from("info_hash"), BEncode::String(ib));
                // TODO: Consider changing this once uTP is implemented
                args.insert(
                    String::from("implied_port"),
                    BEncode::Int(if implied_port { 1 } else { 0 }),
                );
                args.insert(String::from("port"), BEncode::Int(i64::from(port)));
                args.insert(String::from("token"), BEncode::String(token));

                b.insert(String::from("a"), BEncode::Dict(args));
            }
        }
        BEncode::Dict(b).encode_to_buf()
    }

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let b: BEncode = bencode::decode_buf(buf)
            .chain_err(|| ErrorKind::InvalidRequest("Invalid BEncoded data"))?;
        let mut d = b
            .into_dict()
            .ok_or_else(|| ErrorKind::InvalidRequest("Invalid BEncoded data(must be dict)"))?;
        let transaction = d.remove("t").and_then(|b| b.into_bytes()).ok_or_else(|| {
            ErrorKind::InvalidRequest("Invalid BEncoded data(dict must have t field)")
        })?;
        let version = d.remove("v").and_then(|b| b.into_string());
        let y = d.remove("y").and_then(|b| b.into_string()).ok_or_else(|| {
            Error::from(ErrorKind::InvalidRequest(
                "Invalid BEncoded data(dict must have y field)",
            ))
        })?;
        if y != "q" {
            return Err(Error::from(ErrorKind::InvalidRequest(
                "Invalid BEncoded data(request must have y: q field)",
            )));
        }
        let q = d.remove("q").and_then(|b| b.into_string()).ok_or_else(|| {
            Error::from(ErrorKind::InvalidRequest(
                "Invalid BEncoded data(dict must have q field)",
            ))
        })?;
        let mut a = d.remove("a").and_then(|b| b.into_dict()).ok_or_else(|| {
            Error::from(ErrorKind::InvalidRequest(
                "Invalid BEncoded data(dict must have a field)",
            ))
        })?;
        let id = a
            .remove("id")
            .and_then(|b| b.into_bytes())
            .and_then(|b| b.get(0..20).map(BigUint::from_bytes_be))
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidRequest(
                    "Invalid BEncoded data(ping must have id field)",
                ))
            })?;
        let kind = match &q[..] {
            "ping" => RequestKind::Ping(id),
            "find_node" => {
                let target = a
                    .remove("target")
                    .and_then(|b| b.into_bytes())
                    .and_then(|b| b.get(0..20).map(BigUint::from_bytes_be))
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidRequest(
                            "Invalid BEncoded data(find_node must have target field)",
                        ))
                    })?;
                RequestKind::FindNode { id, target }
            }
            "get_peers" => {
                let mut hash = [0u8; 20];
                a.remove("info_hash")
                    .and_then(|b| b.into_bytes())
                    .and_then(|b| {
                        if b.len() != 20 {
                            return None;
                        }
                        hash.copy_from_slice(&b[..20]);
                        Some(())
                    })
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidRequest(
                            "Invalid BEncoded data(get_peers must have hash field)",
                        ))
                    })?;
                RequestKind::GetPeers { id, hash }
            }
            "announce_peer" => {
                let mut hash = [0u8; 20];
                a.remove("info_hash")
                    .and_then(|b| b.into_bytes())
                    .and_then(|b| {
                        if b.len() != 20 {
                            return None;
                        }
                        hash.copy_from_slice(&b[..20]);
                        Some(())
                    })
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidRequest(
                            "Invalid BEncoded data(announce_peer must have hash field)",
                        ))
                    })?;
                let implied_port = a
                    .remove("implied_port")
                    .and_then(|b| b.into_int())
                    .map(|b| b > 0)
                    .unwrap_or(false);
                let port = a
                    .remove("port")
                    .and_then(|b| b.into_int())
                    .and_then(|b| {
                        if b > 65_535 || b < 0 {
                            None
                        } else {
                            Some(b as u16)
                        }
                    })
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidRequest(
                            "Invalid BEncoded data(announce_peer must have port field)",
                        ))
                    })?;
                let token = a
                    .remove("token")
                    .and_then(|b| b.into_bytes())
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidRequest(
                            "Invalid BEncoded data(announce_peer must have port field)",
                        ))
                    })?;
                RequestKind::AnnouncePeer {
                    id,
                    hash,
                    implied_port,
                    port,
                    token,
                }
            }
            _ => {
                return Err(ErrorKind::InvalidRequest(
                    "Invalid BEncoded data(request must be a valid query type)",
                )
                .into());
            }
        };
        Ok(Request {
            transaction,
            version,
            kind,
        })
    }
}

impl Response {
    pub fn id(transaction: Vec<u8>, id: ID) -> Self {
        Response {
            transaction,
            kind: ResponseKind::ID(id),
        }
    }

    pub fn find_node(transaction: Vec<u8>, id: ID, nodes: Vec<Node>) -> Self {
        Response {
            transaction,
            kind: ResponseKind::FindNode { id, nodes },
        }
    }

    pub fn peers(transaction: Vec<u8>, id: ID, token: Vec<u8>, nodes: Vec<SocketAddr>) -> Self {
        Response {
            transaction,
            kind: ResponseKind::GetPeers {
                id,
                token,
                values: nodes,
                nodes: Vec::new(),
            },
        }
    }

    pub fn nodes(transaction: Vec<u8>, id: ID, token: Vec<u8>, nodes: Vec<Node>) -> Self {
        Response {
            transaction,
            kind: ResponseKind::GetPeers {
                id,
                token,
                nodes,
                values: Vec::new(),
            },
        }
    }

    pub fn error(transaction: Vec<u8>, error: ErrorKind) -> Self {
        Response {
            transaction,
            kind: ResponseKind::Error(error),
        }
    }

    pub fn encode(self) -> Vec<u8> {
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
            ResponseKind::GetPeers {
                id,
                token,
                nodes,
                values,
            } => {
                args.insert(String::from("id"), BEncode::String(id.to_bytes_be()));
                args.insert(String::from("token"), BEncode::String(token));
                let mut values_b = Vec::new();
                for addr in values {
                    values_b.push(BEncode::String(addr_to_bytes(&addr).to_vec()));
                }
                args.insert(String::from("values"), BEncode::List(values_b));

                let mut nodes_b = Vec::new();
                for node in nodes {
                    nodes_b.extend(node.to_bytes())
                }
                args.insert(String::from("nodes"), BEncode::String(nodes_b));
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

    pub fn decode(buf: &[u8]) -> Result<Self> {
        let b: BEncode = bencode::decode_buf(buf)
            .chain_err(|| ErrorKind::InvalidResponse("Invalid BEncoded data"))?;
        let mut d = b.into_dict().ok_or_else(|| {
            Error::from(ErrorKind::InvalidResponse(
                "Invalid BEncoded data(must be dict)",
            ))
        })?;
        let transaction = d.remove("t").and_then(|b| b.into_bytes()).ok_or_else(|| {
            Error::from(ErrorKind::InvalidResponse(
                "Invalid BEncoded data(dict must have t field)",
            ))
        })?;
        let y = d.remove("y").and_then(|b| b.into_string()).ok_or_else(|| {
            Error::from(ErrorKind::InvalidResponse(
                "Invalid BEncoded data(dict must have y field)",
            ))
        })?;
        match &y[..] {
            "e" => {
                let mut e = d.remove("e").and_then(|b| b.into_list()).ok_or_else(|| {
                    Error::from(ErrorKind::InvalidResponse(
                        "Invalid BEncoded data(error resp must have e field)",
                    ))
                })?;
                if e.len() != 2 {
                    return Err(ErrorKind::InvalidResponse(
                        "Invalid BEncoded data(e field must have two terms)",
                    )
                    .into());
                }
                let code = e.remove(0).into_int().ok_or_else(|| {
                    Error::from(ErrorKind::InvalidResponse(
                        "Invalid BEncoded data(e field must start with integer code)",
                    ))
                })?;
                let msg = e.remove(0).into_string().ok_or_else(|| {
                    Error::from(ErrorKind::InvalidResponse(
                        "Invalid BEncoded data(e field must end with string data)",
                    ))
                })?;
                let err = match code {
                    201 => ErrorKind::Generic(msg),
                    202 => ErrorKind::Server(msg),
                    203 => ErrorKind::Protocol(msg),
                    204 => ErrorKind::MethodUnknown(msg),
                    _ => {
                        return Err(ErrorKind::InvalidResponse(
                            "Invalid BEncoded data(invalid error code)",
                        )
                        .into())
                    }
                };
                Ok(Response {
                    transaction,
                    kind: ResponseKind::Error(err),
                })
            }
            "r" => {
                let mut r = d.remove("r").and_then(|b| b.into_dict()).ok_or_else(|| {
                    Error::from(ErrorKind::InvalidResponse(
                        "Invalid BEncoded data(resp must have r field)",
                    ))
                })?;

                let id = r
                    .remove("id")
                    .and_then(|b| b.into_bytes())
                    .and_then(|b| b.get(0..20).map(BigUint::from_bytes_be))
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidResponse(
                            "Invalid BEncoded data(response must have id)",
                        ))
                    })?;

                let kind = if let Some(token) = r.remove("token").and_then(|b| b.into_bytes()) {
                    let mut values = Vec::new();
                    if let Some(addrs) = r.remove("values").and_then(|b| b.into_list()) {
                        for addr in addrs {
                            if let Some(data) = addr.into_bytes() {
                                if data.len() == 6 {
                                    values.push(bytes_to_addr(&data));
                                }
                            }
                        }
                    }
                    let mut nodes = Vec::new();
                    if let Some(ns) = r.remove("nodes").and_then(|b| b.into_bytes()) {
                        for n in ns.chunks(26) {
                            if n.len() == 26 {
                                nodes.push(Node::new(n));
                            }
                        }
                    }
                    ResponseKind::GetPeers {
                        id,
                        token,
                        nodes,
                        values,
                    }
                } else if let Some(ns) = r.remove("nodes").and_then(|b| b.into_bytes()) {
                    let mut nodes = Vec::new();
                    for n in ns.chunks(26) {
                        if n.len() == 26 {
                            nodes.push(Node::new(n));
                        }
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
        Node {
            id,
            addr: bytes_to_addr(&data[20..]),
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut data = self.id.to_bytes_be();
        data.extend_from_slice(&addr_to_bytes(&self.addr)[..]);
        data
    }
}

#[cfg(test)]
mod tests {
    use super::{Request, Response, ResponseKind};
    use num_bigint::BigUint;

    #[test]
    fn test_decode_id_resp() {
        let r = Vec::from(&include_bytes!("test/id")[..]);
        let d = Response::decode(&r).unwrap();
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
        let r = Vec::from(&include_bytes!("test/id")[..]);
        let d = Response::decode(&r).unwrap();
        let e = d.encode();
        assert_eq!(e, r);
    }

    #[test]
    fn test_encode_decode_req_ping() {
        let r = Vec::from(&include_bytes!("test/ping")[..]);
        let d = Request::decode(&r).unwrap();
        assert_eq!(d.encode(), r);
    }

    #[test]
    fn test_encode_decode_req_find() {
        let r = Vec::from(&include_bytes!("test/find")[..]);
        let d = Request::decode(&r).unwrap();
        assert_eq!(d.encode(), r);
    }

    #[test]
    fn test_encode_decode_req_get() {
        let r = Vec::from(&include_bytes!("test/get")[..]);
        let d = Request::decode(&r).unwrap();
        assert_eq!(d.encode(), r);
    }

    #[test]
    fn test_encode_decode_req_annnounce() {
        let r = Vec::from(&include_bytes!("test/announce")[..]);
        let d = Request::decode(&r).unwrap();
        assert_eq!(
            String::from_utf8(d.encode()).unwrap(),
            String::from_utf8(r).unwrap()
        );
    }
}
