use super::{proto, BUCKET_MAX, ID, MAX_BUCKETS, MIN_BOOTSTRAP_BKTS, TX_TIMEOUT_SECS};
use crate::tracker;
use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use chrono::{DateTime, Utc};
use num_bigint::BigUint;
use rand::{self, Rng};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::{cmp, mem};

const MAX_SEARCH_DEPTH: u8 = 5;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoutingTable {
    id: ID,
    buckets: Vec<Bucket>,
    last_resp_recvd: DateTime<Utc>,
    last_req_recvd: DateTime<Utc>,
    last_token_refresh: DateTime<Utc>,
    last_tick: DateTime<Utc>,
    transactions: HashMap<u32, Transaction>,
    torrents: HashMap<[u8; 20], Torrent>,
    bootstrapping: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Transaction {
    created: DateTime<Utc>,
    kind: TransactionKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum TransactionKind {
    Initialization,
    Query(ID),
    TSearch {
        id: ID,
        torrent: usize,
        hash: [u8; 20],
        depth: u8,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Torrent {
    peers: Vec<(ID, SocketAddr)>,
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
    token: Vec<u8>,
    prev_token: Vec<u8>,
    rem_token: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeState {
    Good,
    Questionable(usize),
    Bad,
}

impl RoutingTable {
    pub fn new() -> RoutingTable {
        let mut id = [0u8; 20];
        let mut rng = rand::thread_rng();
        for i in &mut id {
            *i = rng.gen::<u8>();
        }

        RoutingTable {
            buckets: vec![Bucket::new(BigUint::from(0u8), id_from_pow(160))],
            last_resp_recvd: Utc::now(),
            last_req_recvd: Utc::now(),
            last_token_refresh: Utc::now(),
            last_tick: Utc::now(),
            id: BigUint::from_bytes_be(&id),
            transactions: HashMap::new(),
            torrents: HashMap::new(),
            bootstrapping: true,
        }
    }

    pub fn init(&mut self) -> Vec<(proto::Request, SocketAddr)> {
        self.refresh_tokens()
    }

    pub fn deserialize(data: &[u8]) -> Option<RoutingTable> {
        bincode::deserialize(data).ok().map(|rt: RoutingTable| {
            info!(
                "Loaded {} dht buckets, {} nodes",
                rt.buckets.len(),
                rt.buckets.iter().map(|buk| buk.nodes.len()).sum::<usize>()
            );
            rt
        })
    }

    pub fn add_addr(&mut self, addr: SocketAddr) -> (proto::Request, SocketAddr) {
        let tx = self.new_init_tx();
        (proto::Request::ping(tx, self.id.clone()), addr)
    }

    pub fn get_peers(
        &mut self,
        torrent: usize,
        hash: [u8; 20],
    ) -> Vec<(proto::Request, SocketAddr)> {
        let tid = BigUint::from_bytes_be(&hash[..]);
        let idx = self.bucket_idx(&tid);
        let mut nodes: Vec<proto::Node> = Vec::new();

        for node in &self.buckets[idx].nodes {
            nodes.push(node.into());
        }

        let mut reqs = Vec::new();
        for node in nodes {
            let tx = self.new_tsearch_tx(node.id, torrent, hash, 0);
            let req = proto::Request::get_peers(tx, self.id.clone(), hash);
            reqs.push((req, node.addr));
        }
        reqs
    }

    pub fn announce(&mut self, hash: [u8; 20]) -> Vec<(proto::Request, SocketAddr)> {
        let mut nodes: Vec<(proto::Node, Vec<u8>)> = Vec::new();
        for bucket in &self.buckets {
            for node in &bucket.nodes {
                if let Some(ref tok) = node.rem_token {
                    nodes.push((node.into(), tok.clone()))
                }
            }
        }

        let mut reqs = Vec::new();
        for (node, tok) in nodes {
            let tx = self.new_query_tx(node.id);
            let req = proto::Request::announce(tx, self.id.clone(), hash, tok);
            reqs.push((req, node.addr));
        }
        reqs
    }

    pub fn handle_req(&mut self, req: proto::Request, mut addr: SocketAddr) -> proto::Response {
        self.last_req_recvd = Utc::now();
        match req.kind {
            // TODO: Consider adding the node if we don't have it?
            proto::RequestKind::Ping(id) => {
                if self.contains_id(&id) {
                    self.get_node_mut(&id).update();
                }
                proto::Response::id(req.transaction, self.id.clone())
            }
            proto::RequestKind::FindNode { id, target } => {
                if self.contains_id(&id) {
                    self.get_node_mut(&id).update();
                }
                let mut nodes = Vec::new();
                if self.contains_id(&target) {
                    nodes.push(self.get_node(&target).into())
                } else {
                    let b = self.bucket_idx(&target);
                    for node in &self.buckets[b].nodes {
                        nodes.push(node.into());
                    }
                }
                proto::Response::find_node(req.transaction, self.id.clone(), nodes)
            }
            proto::RequestKind::AnnouncePeer {
                id,
                implied_port,
                hash,
                port,
                token,
            } => {
                if !self.contains_id(&id) {
                    return proto::Response::error(
                        req.transaction,
                        proto::ErrorKind::Protocol("Unregistered peer!".to_owned()),
                    );
                }
                {
                    let node = self.get_node_mut(&id);
                    if !node.token_valid(&token) {
                        return proto::Response::error(
                            req.transaction,
                            proto::ErrorKind::Protocol("Bad token!".to_owned()),
                        );
                    }
                    node.update();
                }
                self.torrents
                    .entry(hash)
                    .or_insert(Torrent { peers: Vec::new() });
                if !implied_port {
                    addr.set_port(port);
                }
                self.torrents.get_mut(&hash).unwrap().peers.push((id, addr));
                proto::Response::id(req.transaction, self.id.clone())
            }
            proto::RequestKind::GetPeers { id, hash } => {
                if !self.contains_id(&id) {
                    let n = Node::new(id.clone(), addr);
                    if self.add_node(n).is_err() {
                        // This will be processed immediately after.
                    }
                }
                let token = if !self.contains_id(&id) {
                    return proto::Response::error(
                        req.transaction,
                        proto::ErrorKind::Protocol("Unregistered peer!".to_owned()),
                    );
                } else {
                    self.get_node(&id).token.clone()
                };
                if let Some(t) = self.torrents.get(&hash) {
                    proto::Response::peers(
                        req.transaction,
                        self.id.clone(),
                        token,
                        t.peers.iter().map(|p| p.1).collect(),
                    )
                } else {
                    let mut nodes = Vec::new();
                    let b = self.bucket_idx(&BigUint::from_bytes_be(&hash[..]));
                    for node in &self.buckets[b].nodes {
                        nodes.push(node.into());
                    }
                    proto::Response::nodes(req.transaction, self.id.clone(), token, nodes)
                }
            }
        }
    }

    pub fn handle_resp(
        &mut self,
        resp: proto::Response,
        addr: SocketAddr,
    ) -> Result<tracker::Response, Vec<(proto::Request, SocketAddr)>> {
        self.last_resp_recvd = Utc::now();
        let mut reqs = Vec::new();
        if resp.transaction.len() < 4 {
            return Err(reqs);
        }
        let tid = BigEndian::read_u32(&resp.transaction[..]);
        let tx = if let Some(tx) = self.transactions.remove(&tid) {
            tx
        } else {
            return Err(reqs);
        };

        match (tx.kind, resp.kind) {
            (TransactionKind::Initialization, proto::ResponseKind::ID(id)) => {
                let mut n = Node::new(id.clone(), addr);
                n.update();
                if self.add_node(n).is_ok() && self.bootstrapping {
                    let tx = self.new_query_tx(id);
                    reqs.push((
                        proto::Request::find_node(tx, self.id.clone(), self.id.clone()),
                        addr,
                    ));
                }
            }

            (TransactionKind::Query(ref id1), proto::ResponseKind::ID(ref id2)) if id1 == id2 => {
                if !self.contains_id(id1) {
                    return Err(reqs);
                }
                self.get_node_mut(id1).update();
                if self.bootstrapping {
                    let tx = self.new_query_tx(id1.clone());
                    reqs.push((
                        proto::Request::find_node(tx, self.id.clone(), self.id.clone()),
                        addr,
                    ));
                }
            }

            (
                TransactionKind::Query(ref id1),
                proto::ResponseKind::FindNode {
                    id: ref id2,
                    ref mut nodes,
                },
            ) if id1 == id2 => {
                if !self.contains_id(id1) {
                    return Err(reqs);
                }
                self.get_node_mut(id1).update();
                for node in nodes.drain(..) {
                    if !self.contains_id(&node.id) {
                        let id = node.id.clone();
                        let addr = node.addr;
                        if self.add_node(node.into()).is_ok() {
                            let tx = self.new_query_tx(id);
                            reqs.push((proto::Request::ping(tx, self.id.clone()), addr));
                        }
                    }
                }
            }

            // Token refresh query
            (
                TransactionKind::Query(ref id1),
                proto::ResponseKind::GetPeers {
                    id: ref id2,
                    ref mut token,
                    ..
                },
            ) if id1 == id2 => {
                if !self.contains_id(id1) {
                    return Err(reqs);
                }
                let node = self.get_node_mut(id1);
                node.update();
                if let Some(ref mut rt) = node.rem_token {
                    mem::swap(rt, token);
                } else {
                    node.rem_token = Some(token.clone());
                }
            }

            (
                TransactionKind::TSearch {
                    id: ref id1,
                    torrent,
                    hash,
                    depth,
                },
                proto::ResponseKind::GetPeers {
                    id: ref id2,
                    ref mut values,
                    ref mut nodes,
                    ref mut token,
                },
            ) if id1 == id2 => {
                if self.contains_id(id1) {
                    let node = self.get_node_mut(id1);
                    node.update();
                    if let Some(ref mut rt) = node.rem_token {
                        mem::swap(rt, token);
                    } else {
                        node.rem_token = Some(token.clone());
                    }
                }

                if !values.is_empty() {
                    return Ok(tracker::Response::DHT {
                        tid: torrent,
                        peers: mem::replace(values, vec![]),
                    });
                }

                if depth < MAX_SEARCH_DEPTH {
                    for node in nodes.drain(..) {
                        let id = node.id.clone();
                        let addr = node.addr;
                        if !self.contains_id(&node.id) {
                            let tx = self.new_tsearch_tx(id.clone(), torrent, hash, depth + 1);
                            reqs.push((proto::Request::get_peers(tx, self.id.clone(), hash), addr));
                        }
                    }
                }
            }

            (TransactionKind::Query(id), proto::ResponseKind::Error(_)) => {
                if !self.contains_id(&id) {
                    return Err(reqs);
                }
                self.get_node_mut(&id).update();
            }

            // Mismatched IDs
            (TransactionKind::Query(id), proto::ResponseKind::ID(_))
            | (TransactionKind::Query(id), proto::ResponseKind::FindNode { .. })
            | (TransactionKind::Query(id), proto::ResponseKind::GetPeers { .. })
            | (TransactionKind::TSearch { id, .. }, proto::ResponseKind::GetPeers { .. })
            | (TransactionKind::TSearch { id, .. }, _) => {
                self.remove_node(&id);
            }

            // Bad response to initialization ping
            (TransactionKind::Initialization, _) => {
                // No need to do anything here, we haven't added the node to the routing
                // table yet.
            }
        }
        Err(reqs)
    }

    pub fn tick(&mut self) -> Vec<(proto::Request, SocketAddr)> {
        let mut reqs = Vec::new();
        let dur = Utc::now().signed_duration_since(self.last_tick);
        if dur.num_seconds() < 10 {
            return reqs;
        }
        self.last_tick = Utc::now();

        let mut nodes_to_ping: Vec<proto::Node> = Vec::new();
        if self.is_bootstrapped() && self.bootstrapping {
            self.bootstrapping = false;
        }

        self.transactions.retain(|_, tx| {
            Utc::now().signed_duration_since(tx.created).num_seconds() < TX_TIMEOUT_SECS
        });

        let dur = Utc::now().signed_duration_since(self.last_token_refresh);
        let tok_refresh = dur.num_minutes() > 5;

        for bucket in &mut self.buckets {
            for node in &mut bucket.nodes {
                if tok_refresh {
                    node.new_token();
                }
                let dur = Utc::now().signed_duration_since(node.last_updated);
                if dur.num_minutes() > 15 {
                    if node.good() {
                        node.state = NodeState::Questionable(1);
                        nodes_to_ping.push((&*node).into());
                    } else if let NodeState::Questionable(1) = node.state {
                        node.state = NodeState::Questionable(2);
                        nodes_to_ping.push((&*node).into());
                    } else {
                        node.state = NodeState::Bad;
                    }
                }
            }
        }
        for node in nodes_to_ping {
            let tx = self.new_query_tx(node.id);
            reqs.push((proto::Request::ping(tx, self.id.clone()), node.addr));
        }
        let buckets = &self.buckets;
        self.torrents.retain(|_, t| {
            t.peers.retain(|p| {
                let idx = RoutingTable::bucket_idx_(&p.0, buckets);
                buckets[idx].contains(&p.0)
            });
            !t.peers.is_empty()
        });
        reqs
    }

    pub fn serialize(&self) -> Vec<u8> {
        bincode::serialize(self).unwrap()
    }

    pub fn is_bootstrapped(&self) -> bool {
        self.buckets.len() >= MIN_BOOTSTRAP_BKTS
    }

    /// Send a bogus get_peers query and internally refresh our token.
    fn refresh_tokens(&mut self) -> Vec<(proto::Request, SocketAddr)> {
        let mut nodes: Vec<proto::Node> = Vec::new();
        for bucket in &mut self.buckets {
            for node in &mut bucket.nodes {
                node.new_token();
                nodes.push((&*node).into());
            }
        }

        let mut reqs = Vec::new();
        for node in nodes {
            let tx = self.new_query_tx(node.id);
            let req = proto::Request::get_peers(tx, self.id.clone(), [0xBEu8; 20]);
            reqs.push((req, node.addr));
        }
        reqs
    }

    fn get_node_mut(&mut self, id: &ID) -> &mut Node {
        let idx = self.bucket_idx(id);
        let bidx = self.buckets[idx].idx_of(id).unwrap();
        &mut self.buckets[idx].nodes[bidx]
    }

    fn get_node(&self, id: &ID) -> &Node {
        let idx = self.bucket_idx(id);
        let bidx = self.buckets[idx].idx_of(id).unwrap();
        &self.buckets[idx].nodes[bidx]
    }

    fn contains_id(&self, id: &ID) -> bool {
        let idx = self.bucket_idx(id);
        self.buckets[idx].contains(id)
    }

    fn new_init_tx(&mut self) -> Vec<u8> {
        let mut tb = Vec::new();
        let tid = rand::random::<u32>();
        tb.write_u32::<BigEndian>(tid).unwrap();
        self.transactions.insert(
            tid,
            Transaction {
                created: Utc::now(),
                kind: TransactionKind::Initialization,
            },
        );
        tb
    }

    fn new_query_tx(&mut self, id: ID) -> Vec<u8> {
        let mut tb = Vec::new();
        let tid = rand::random::<u32>();
        tb.write_u32::<BigEndian>(tid).unwrap();
        self.transactions.insert(
            tid,
            Transaction {
                created: Utc::now(),
                kind: TransactionKind::Query(id),
            },
        );
        tb
    }

    fn new_tsearch_tx(&mut self, id: ID, torrent: usize, hash: [u8; 20], depth: u8) -> Vec<u8> {
        let mut tb = Vec::new();
        let tid = rand::random::<u32>();
        tb.write_u32::<BigEndian>(tid).unwrap();
        self.transactions.insert(
            tid,
            Transaction {
                created: Utc::now(),
                kind: TransactionKind::TSearch {
                    id,
                    torrent,
                    hash,
                    depth,
                },
            },
        );
        tb
    }

    fn add_node(&mut self, node: Node) -> Result<(), ()> {
        let idx = self.bucket_idx(&node.id);
        if self.buckets[idx].full() {
            if self.buckets[idx].could_hold(&self.id) && self.buckets.len() < MAX_BUCKETS {
                self.split_bucket(idx);
                self.add_node(node)
            } else {
                Err(())
            }
        } else {
            self.buckets[idx].add_node(node);
            Ok(())
        }
    }

    fn remove_node(&mut self, id: &ID) {
        let idx = self.bucket_idx(id);
        if let Some(i) = self.buckets[idx].idx_of(id) {
            self.buckets[idx].nodes.remove(i);
        }
    }

    fn split_bucket(&mut self, idx: usize) {
        let midpoint = self.buckets[idx].midpoint();
        let mut nb;
        {
            let pb = &mut self.buckets[idx];
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

    fn bucket_idx(&self, id: &ID) -> usize {
        RoutingTable::bucket_idx_(id, &self.buckets)
    }

    fn bucket_idx_(id: &ID, buckets: &[Bucket]) -> usize {
        buckets
            .binary_search_by(|bucket| {
                if bucket.could_hold(id) {
                    cmp::Ordering::Equal
                } else {
                    bucket.start.cmp(id)
                }
            })
            .unwrap()
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

    fn add_node(&mut self, mut node: Node) {
        if self.nodes.len() < BUCKET_MAX {
            self.nodes.push(node);
        } else {
            for n in &mut self.nodes {
                if !n.good() {
                    mem::swap(n, &mut node);
                    break;
                }
            }
        }
    }

    fn could_hold(&self, id: &ID) -> bool {
        &self.start <= id && id < &self.end
    }

    fn full(&self) -> bool {
        self.nodes.len() >= BUCKET_MAX && self.nodes.iter().all(|n| n.good())
    }

    fn midpoint(&self) -> ID {
        self.start.clone() + (&self.end - &self.start) / BigUint::from(2u8)
    }

    fn contains(&self, id: &ID) -> bool {
        self.idx_of(id).is_some()
    }

    fn idx_of(&self, id: &ID) -> Option<usize> {
        self.nodes.iter().position(|node| &node.id == id)
    }
}

impl Node {
    fn new(id: ID, addr: SocketAddr) -> Node {
        let token = Node::create_token();
        Node {
            id,
            state: NodeState::Bad,
            addr,
            last_updated: Utc::now(),
            prev_token: token.clone(),
            rem_token: None,
            token,
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

    fn token_valid(&self, token: &[u8]) -> bool {
        token == &self.token[..] || token == &self.prev_token[..]
    }

    fn create_token() -> Vec<u8> {
        let mut tok = Vec::new();
        let mut rng = rand::thread_rng();
        for _ in 0..20 {
            tok.push(rng.gen::<u8>());
        }
        tok
    }

    fn update(&mut self) {
        self.state = NodeState::Good;
        self.last_updated = Utc::now();
    }
}

impl From<proto::Node> for Node {
    fn from(node: proto::Node) -> Self {
        Node::new(node.id, node.addr)
    }
}

impl<'a> Into<proto::Node> for &'a Node {
    fn into(self) -> proto::Node {
        proto::Node {
            id: self.id.clone(),
            addr: self.addr,
        }
    }
}

/// creates an ID of value 2^(pow)
fn id_from_pow(pow: usize) -> ID {
    let mut id = [0u8; 21];
    let idx = 20 - pow / 8;
    let offset = pow % 8;
    let block = id[idx];
    id[idx] = block | (1 << offset);
    BigUint::from_bytes_be(&id)
}

#[cfg(test)]
mod tests {
    use super::{id_from_pow, Bucket, Node, RoutingTable};
    use num_bigint::BigUint;

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
        let b = Bucket::new(BigUint::from(0u8), id_from_pow(160));
        assert_eq!(b.midpoint(), id_from_pow(159));
    }

    #[test]
    fn test_bucket_split_far() {
        let mut rt = RoutingTable::new();
        rt.buckets[0].nodes = vec![Node::new_test(id_from_pow(100)); 8];
        rt.split_bucket(0);
        assert_eq!(rt.buckets[0].nodes.len(), 8);
        assert_eq!(rt.buckets[1].nodes.len(), 0);
    }

    #[test]
    fn test_bucket_split_close() {
        let mut rt = RoutingTable::new();
        rt.buckets[0].nodes = vec![Node::new_test(id_from_pow(159)); 8];
        rt.split_bucket(0);
        assert_eq!(rt.buckets[0].nodes.len(), 0);
        assert_eq!(rt.buckets[1].nodes.len(), 8);
    }
}
