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
        b.insert(b"t".to_vec(), BEncode::String(self.transaction));
        b.insert(b"y".to_vec(), BEncode::from_str("q"));
        if let Some(v) = self.version {
            b.insert(b"v".to_vec(), BEncode::from_str(&v));
        }
        match self.kind {
            RequestKind::Ping(id) => {
                b.insert(b"q".to_vec(), BEncode::from_str("ping"));

                let mut args = BTreeMap::new();
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));

                b.insert(b"a".to_vec(), BEncode::Dict(args));
            }
            RequestKind::FindNode { id, target } => {
                b.insert(b"q".to_vec(), BEncode::from_str("find_node"));

                let mut args = BTreeMap::new();
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
                args.insert(b"target".to_vec(), BEncode::String(target.to_bytes_be()));

                b.insert(b"a".to_vec(), BEncode::Dict(args));
            }
            RequestKind::GetPeers { id, hash } => {
                b.insert(b"q".to_vec(), BEncode::from_str("get_peers"));

                let mut args = BTreeMap::new();
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(b"info_hash".to_vec(), BEncode::String(ib));

                b.insert(b"a".to_vec(), BEncode::Dict(args));
            }
            RequestKind::AnnouncePeer {
                id,
                hash,
                token,
                port,
                implied_port,
            } => {
                b.insert(b"q".to_vec(), BEncode::from_str("announce_peer"));
                let mut args = BTreeMap::new();
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
                let ib = Vec::from(&hash[..]);
                args.insert(b"info_hash".to_vec(), BEncode::String(ib));
                // TODO: Consider changing this once uTP is implemented
                args.insert(
                    b"implied_port".to_vec(),
                    BEncode::Int(if implied_port { 1 } else { 0 }),
                );
                args.insert(b"port".to_vec(), BEncode::Int(i64::from(port)));
                args.insert(b"token".to_vec(), BEncode::String(token));

                b.insert(b"a".to_vec(), BEncode::Dict(args));
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
        let transaction = d
            .remove(b"t".as_ref())
            .and_then(|b| b.into_bytes())
            .ok_or_else(|| {
                ErrorKind::InvalidRequest("Invalid BEncoded data(dict must have t field)")
            })?;
        let version = d.remove(b"v".as_ref()).and_then(|b| b.into_string());
        let y = d
            .remove(b"y".as_ref())
            .and_then(|b| b.into_string())
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidRequest(
                    "Invalid BEncoded data(dict must have y field)",
                ))
            })?;
        if y != "q" {
            return Err(Error::from(ErrorKind::InvalidRequest(
                "Invalid BEncoded data(request must have y: q field)",
            )));
        }
        let q = d
            .remove(b"q".as_ref())
            .and_then(|b| b.into_string())
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidRequest(
                    "Invalid BEncoded data(dict must have q field)",
                ))
            })?;
        let mut a = d
            .remove(b"a".as_ref())
            .and_then(|b| b.into_dict())
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidRequest(
                    "Invalid BEncoded data(dict must have a field)",
                ))
            })?;
        let id = a
            .remove(b"id".as_ref())
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
                    .remove(b"target".as_ref())
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
                a.remove(b"info_hash".as_ref())
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
                a.remove(b"info_hash".as_ref())
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
                    .remove(b"implied_port".as_ref())
                    .and_then(|b| b.into_int())
                    .map(|b| b > 0)
                    .unwrap_or(false);
                let port = a
                    .remove(b"port".as_ref())
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
                    .remove(b"token".as_ref())
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
        b.insert(b"t".to_vec(), BEncode::String(self.transaction));
        let mut args = BTreeMap::new();
        match self.kind {
            ResponseKind::ID(id) => {
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
            }
            ResponseKind::FindNode { id, nodes } => {
                let mut data = Vec::new();
                for node in nodes {
                    data.extend(node.to_bytes())
                }
                args.insert(b"nodes".to_vec(), BEncode::String(data));
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
            }
            ResponseKind::GetPeers {
                id,
                token,
                nodes,
                values,
            } => {
                args.insert(b"id".to_vec(), BEncode::String(id.to_bytes_be()));
                args.insert(b"token".to_vec(), BEncode::String(token));
                let mut values_b = Vec::new();
                for addr in values {
                    values_b.push(BEncode::String(addr_to_bytes(&addr).to_vec()));
                }
                args.insert(b"values".to_vec(), BEncode::List(values_b));

                let mut nodes_b = Vec::new();
                for node in nodes {
                    nodes_b.extend(node.to_bytes())
                }
                args.insert(b"nodes".to_vec(), BEncode::String(nodes_b));
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
                b.insert(b"e".to_vec(), BEncode::List(err));
            }
        }
        if is_err {
            b.insert(b"y".to_vec(), BEncode::from_str("e"));
        } else {
            b.insert(b"y".to_vec(), BEncode::from_str("r"));
            b.insert(b"r".to_vec(), BEncode::Dict(args));
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
        let transaction = d
            .remove(b"t".as_ref())
            .and_then(|b| b.into_bytes())
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidResponse(
                    "Invalid BEncoded data(dict must have t field)",
                ))
            })?;
        let y = d
            .remove(b"y".as_ref())
            .and_then(|b| b.into_string())
            .ok_or_else(|| {
                Error::from(ErrorKind::InvalidResponse(
                    "Invalid BEncoded data(dict must have y field)",
                ))
            })?;
        match &y[..] {
            "e" => {
                let mut e = d
                    .remove(b"e".as_ref())
                    .and_then(|b| b.into_list())
                    .ok_or_else(|| {
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
                let mut r = d
                    .remove(b"r".as_ref())
                    .and_then(|b| b.into_dict())
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidResponse(
                            "Invalid BEncoded data(resp must have r field)",
                        ))
                    })?;

                let id = r
                    .remove(b"id".as_ref())
                    .and_then(|b| b.into_bytes())
                    .and_then(|b| b.get(0..20).map(BigUint::from_bytes_be))
                    .ok_or_else(|| {
                        Error::from(ErrorKind::InvalidResponse(
                            "Invalid BEncoded data(response must have id)",
                        ))
                    })?;

                let kind = if let Some(token) =
                    r.remove(b"token".as_ref()).and_then(|b| b.into_bytes())
                {
                    let mut values = Vec::new();
                    if let Some(addrs) = r.remove(b"values".as_ref()).and_then(|b| b.into_list()) {
                        for addr in addrs {
                            if let Some(data) = addr.into_bytes() {
                                if data.len() == 6 {
                                    values.push(bytes_to_addr(&data));
                                }
                            }
                        }
                    }
                    let mut nodes = Vec::new();
                    if let Some(ns) = r.remove(b"nodes".as_ref()).and_then(|b| b.into_bytes()) {
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
                } else if let Some(ns) = r.remove(b"nodes".as_ref()).and_then(|b| b.into_bytes()) {
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
    use super::{Request, Response};
    use platina;

    struct DhtProtoTest;

    impl platina::Testable for DhtProtoTest {
        fn run_testcase(&mut self, case: &mut platina::TestCase) {
            let encoded = case.get_param("dht_msg").unwrap();
            let reencoded;
            if let Some(_) = case.get_param("response") {
                let decoded = Response::decode(encoded.as_bytes());
                case.compare_and_update_param("decoded", &format!("{:#?}", decoded));
                reencoded = decoded.map(Response::encode);
            } else {
                let decoded = Request::decode(encoded.as_bytes());
                case.compare_and_update_param("decoded", &format!("{:#?}", decoded));
                reencoded = decoded.map(Request::encode);
            }
            if let Ok(bytes) = reencoded {
                assert_eq!(&bytes[..], encoded.as_bytes());
            }
        }
    }

    #[test]
    fn test_diff() {
        let mut t = DhtProtoTest;
        platina::TestFile::new("src/tracker/dht/test/proto_test.plat")
            .run_tests(&mut t)
            .unwrap();
    }

    #[test]
    #[ignore]
    fn test_update() {
        let mut t = DhtProtoTest;
        platina::TestFile::new("src/tracker/dht/test/proto_test.plat")
            .run_tests_and_update(&mut t)
            .unwrap();
    }
}
