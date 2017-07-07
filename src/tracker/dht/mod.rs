use std::net::{SocketAddr, UdpSocket};
use std::{cmp, mem};
use chrono::{DateTime, Utc};
use num::bigint::BigUint;
use CONFIG;

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

pub struct Manager {
    table: RoutingTable,
    sock: UdpSocket,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingTable {
    buckets: Vec<Bucket>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Bucket {
    start: ID,
    end: ID,
    last_updated: DateTime<Utc>,
    nodes: Vec<Node>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    id: ID,
    state: NodeState,
    addr: SocketAddr,
    last_updated: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeState {
    Good,
    Questionable,
    Bad,
}

impl RoutingTable {
    fn new() -> RoutingTable {
        RoutingTable {
            buckets: vec![Bucket::new(BigUint::from(0u8), id_from_pow(160))]
        }
    }

    fn add_node(&mut self, node: Node) {
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
            } else if self.buckets[idx].could_hold(&DHT_ID) {
                let midpoint = self.buckets[idx].midpoint();
                self.split_bucket(idx, midpoint);
            } else {
                // Discard, or TODO: add to queue
            }
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
        Node {
            id,
            state: NodeState::Questionable,
            addr,
            last_updated: Utc::now(),
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
