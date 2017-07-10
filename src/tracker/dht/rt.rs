use std::net::SocketAddr;
use std::{cmp, mem};
use std::collections::{HashMap, VecDeque};
use chrono::{DateTime, Utc};
use num::bigint::BigUint;
use rand::{self, Rng};
use super::{ID, Distance, BUCKET_MAX, proto};
use tracker;
use bincode;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingTable {
    buckets: Vec<Bucket>,
    last_resp_recvd: DateTime<Utc>,
    last_req_recvd: DateTime<Utc>,
    last_token_refresh: DateTime<Utc>,
    id: ID,
    transactions: HashMap<u32, Transaction>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Transaction {
    created: DateTime<Utc>,
    kind: TransactionKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum TransactionKind {
    Initialization,
    Query { id: ID },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bucket {
    start: ID,
    end: ID,
    last_updated: DateTime<Utc>,
    queue: VecDeque<proto::Node>,
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    id: ID,
    state: NodeState,
    addr: SocketAddr,
    last_updated: DateTime<Utc>,
    token: Vec<u8>,
    prev_token: Vec<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeState {
    Good,
    Questionable,
    Bad,
}

impl RoutingTable {
    pub fn new() -> RoutingTable {
        let mut id = [0u8; 20];
        let mut rng = rand::thread_rng();
        for i in 0..20 {
            id[i] = rng.gen::<u8>();
        }
        
        RoutingTable {
            buckets: vec![Bucket::new(BigUint::from(0u8), id_from_pow(160))],
            last_resp_recvd: Utc::now(),
            last_req_recvd: Utc::now(),
            last_token_refresh: Utc::now(),
            id: BigUint::from_bytes_be(&id),
            transactions: HashMap::new(),
        }
    }

    pub fn deserialize() -> Option<RoutingTable> {
        None
    }

    pub fn add_addr(&mut self, addr: SocketAddr) -> (proto::Request, SocketAddr) {
        unimplemented!();
    }

    pub fn handle_req(&mut self, req: proto::Request) -> proto::Response {
        unimplemented!();
    }

    pub fn handle_resp(&mut self, resp: proto::Response) -> Result<tracker::Response, Vec<(proto::Request, SocketAddr)>> {
        unimplemented!();
    }

    pub fn tick(&mut self) -> Vec<(proto::Request, SocketAddr)> {
        let mut resps = Vec::new();
        resps
    }

    fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self, bincode::Infinite).unwrap()
    }

    fn bootstrap(&mut self, node: SocketAddr) {
    }

    fn add_node(&mut self, node: proto::Node) {
        let idx = self.buckets.binary_search_by(|bucket| {
            if bucket.could_hold(&node.id) {
                cmp::Ordering::Equal
            } else {
                node.id.cmp(&bucket.start)
            }
        }).unwrap();

        if self.buckets[idx].full() {
            if self.buckets.len() == 1 {
                self.split_bucket(idx, id_from_pow(159));
            } else if self.buckets[idx].could_hold(&self.id) {
                let midpoint = self.buckets[idx].midpoint();
                self.split_bucket(idx, midpoint);
            } else {
                // Discard, or TODO: add to queue
            }
        } else {
        }
    }

    fn split_bucket(&mut self, idx: usize, midpoint: ID) {
        let mut nb;
        {
            let pb = self.buckets.get_mut(idx).unwrap();
            nb = Bucket::new(midpoint.clone(), pb.end.clone());
            pb.end = midpoint;
            let nodes = mem::replace(&mut pb.nodes, Vec::with_capacity(BUCKET_MAX));
            for node in nodes {
                if pb.could_hold(&node.id) {
                    pb.nodes.push(node);
                } else {
                    nb.nodes.push(node);
                }
            }
        }
        self.buckets.insert(idx + 1, nb);
    }
}

impl Bucket {
    fn new(start: ID, end: ID) -> Bucket {
        Bucket {
            start,
            end,
            last_updated: Utc::now(),
            queue: VecDeque::new(),
            nodes: Vec::with_capacity(BUCKET_MAX),
        }
    }

    fn could_hold(&self, id: &ID) -> bool {
        &self.start <= id && id < &self.end
    }

    fn full(&self) -> bool {
        self.nodes.len() == BUCKET_MAX &&
            self.nodes.iter().all(|n| n.good())
    }

    fn midpoint(&self) -> ID {
        self.start.clone() + ((&self.end - &self.start))/BigUint::from(2u8)
    }
}

impl Node {
    fn new(id: ID, addr: SocketAddr) -> Node {
        let token = Node::create_token();
        Node {
            id,
            state: NodeState::Questionable,
            addr,
            last_updated: Utc::now(),
            prev_token: token.clone(),
            token: token,
        }
    }

    #[cfg(test)]
    fn new_test(id: ID) -> Node {
        Node::new(id, "127.0.0.1:0".parse().unwrap())
    }

    fn good(&self) -> bool {
        if let NodeState::Good = self.state {
            true
        } else {
            false
        }
    }

    fn new_token(&mut self) {
        let new_prev = mem::replace(&mut self.token, Node::create_token());
        self.prev_token = new_prev;
    }

    fn token_valid(&self, token: Vec<u8>) -> bool {
        token == self.token || token == self.prev_token
    }

    fn create_token() -> Vec<u8> {
        let mut tok = Vec::new();
        let mut rng = rand::thread_rng();
        for i in 0..20 {
            tok.push(rng.gen::<u8>());
        }
        tok
    }
}

/// creates an ID of value 2^(pow)
fn id_from_pow(pow: usize) -> ID {
    let mut id = [0u8; 21];
    let idx = 20 - pow/8;
    let offset = pow % 8;
    let block = id[idx];
    id[idx] = block | (1 << offset);
    BigUint::from_bytes_be(&id)
}

fn distance(a: &ID, b: &ID) -> Distance {
    a ^ b
}

#[cfg(test)]
mod tests {
    use super::{Bucket, Node, RoutingTable, distance, id_from_pow};
    use num::bigint::BigUint;

    #[test]
    fn test_distance() {
        assert_eq!(distance(&id_from_pow(10), &id_from_pow(10)), BigUint::from(0u8));
    }

    #[test]
    fn test_id_from_pow() {
        assert!(id_from_pow(159) > id_from_pow(158));
        assert_eq!(id_from_pow(1), BigUint::from(2u8));
        assert_eq!(id_from_pow(8), BigUint::from(256u16));
    }

    #[test]
    fn test_bucket_midpoint() {
        let b = Bucket::new(BigUint::from(0u8), BigUint::from(20u8));
        assert_eq!(b.midpoint(), BigUint::from(10u8));
    }

    #[test]
    fn test_bucket_split_far() {
        let mut rt = RoutingTable::new();
        rt.buckets[0].nodes = vec![Node::new_test(id_from_pow(100)); 8];
        rt.split_bucket(0, id_from_pow(159));
        assert_eq!(rt.buckets[0].nodes.len(), 8);
        assert_eq!(rt.buckets[1].nodes.len(), 0);
    }

    #[test]
    fn test_bucket_split_close() {
        let mut rt = RoutingTable::new();
        rt.buckets[0].nodes = vec![Node::new_test(id_from_pow(100)); 8];
        rt.split_bucket(0, id_from_pow(100));
        assert_eq!(rt.buckets[0].nodes.len(), 0);
        assert_eq!(rt.buckets[1].nodes.len(), 8);
    }
}
