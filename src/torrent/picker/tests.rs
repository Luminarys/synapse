use super::Picker;
use std::collections::HashSet;
use std::cell::UnsafeCell;
use torrent::{Bitfield, Peer as TPeer, Info};
use rand::distributions::{IndependentSample, Range};
use rand;

struct Simulation {
    cfg: TestCfg,
    ticks: usize,
    peers: UnsafeCell<Vec<Peer>>,
}

impl Simulation {
    fn new(cfg: TestCfg, picker: Picker) -> Simulation {
        let mut rng = rand::thread_rng();
        let mut peers = Vec::new();
        for i in 0..cfg.peers {
            let connected = rand::sample(&mut rng, 0..cfg.peers as usize, cfg.connect_limit as usize);
            let unchoked = rand::sample(&mut rng, connected.iter().map(|v| *v), cfg.unchoke_limit as usize);
            let peer = Peer {
                picker: picker.clone(),
                connected,
                unchoked,
                unchoked_by: Vec::new(),
                requests: Vec::new(),
                requested_peers: HashSet::new(),
                data: {
                    let mut p = TPeer::test();
                    p.id = i as usize;
                    p.pieces = Bitfield::new(cfg.pieces as u64);
                    p
                }
            };
            peers.push(peer);
        }
        Simulation {
            cfg,
            ticks: 0,
            peers: UnsafeCell::new(peers),
        }
    }

    fn init(&mut self) {
        for i in 0..self.cfg.pieces {
            self.peers()[0].data.pieces.set_bit(i as u64);
        }
        for piece in self.peers()[0].data.pieces.iter() {
            println!("Set {:?}", piece);
        }
        assert!(self.peers()[0].data.pieces.complete());
        for peer in self.peers().iter() {
            for pid in peer.unchoked.iter() {
                self.peers()[*pid].unchoked_by.push(peer.data.id);
            }
        }
    }

    fn run(&mut self) -> usize {
        while let Err(()) = self.tick() {
            self.ticks += 1;
            if self.ticks as u32 >= 3 * (self.cfg.pieces + self.cfg.peers as u32) {
                panic!();
            }
        }
        return self.ticks;
    }

    fn tick(&mut self) -> Result<(), ()> {
        println!("\nTick: {:?}\n", self.ticks);
        let mut rng = rand::thread_rng();
        for peer in self.peers().iter_mut() {
            println!("Handling peer: {:?}", peer.data.id);
            if !peer.requests.is_empty() {
                let req = if true {
                    peer.requests.pop().unwrap()
                } else {
                    let b = Range::new(0, peer.requests.len());
                    peer.requests.remove(b.ind_sample(&mut rng))
                };
                println!("Handling request from: {:?}", req.peer);
                let ref mut received = self.peers()[req.peer];
                received.picker.completed(req.piece, 0);
                received.data.pieces.set_bit(req.piece as u64);
                if received.data.pieces.complete() {
                    for p in self.peers().iter_mut() {
                        if !p.data.pieces.complete() && !p.unchoked_by.contains(&peer.data.id) {
                            p.unchoked_by.push(peer.data.id);
                        }
                    }
                }
                received.requested_peers.remove(&peer.data.id);
                for pid in received.connected.iter() {
                    self.peers()[*pid].picker.piece_available(req.piece);
                }
            }

            for pid in peer.unchoked_by.iter() {
                let ref mut ucp = self.peers()[*pid];
                if peer.data.pieces.usable(&ucp.data.pieces) && !peer.requested_peers.contains(&ucp.data.id) {
                    println!("Making request to: {:?}", ucp.data.id);
                    if let Some((piece, _)) = peer.picker.pick(&ucp.data) {
                        ucp.requests.push(Request { peer: peer.data.id, piece });
                        peer.requested_peers.insert(ucp.data.id);
                    }
                }
            }
        }
        let inc = self.peers().iter().filter(|p| !p.data.pieces.complete()).collect::<Vec<_>>();
        if  inc.is_empty() {
            Ok(())
        } else {
            println!("{:?}", inc[0]);
            Err(())
        }
    }

    fn peers<'f>(&self) -> &'f mut Vec<Peer> {
        unsafe {
            self.peers.get().as_mut().unwrap()
        }
    }
}

#[derive(Debug)]
struct Peer {
    data: TPeer,
    picker: Picker,
    connected: Vec<usize>,
    unchoked: Vec<usize>,
    unchoked_by: Vec<usize>,
    requests: Vec<Request>,
    requested_peers: HashSet<usize>,
}

#[derive(Debug)]
struct Request {
    peer: usize,
    piece: u32,
}

#[derive(Clone)]
struct TestCfg {
    pieces: u32,
    peers: u16,
    unchoke_limit: u8,
    connect_limit: u8,
}

/// Tests the general efficiency of a piece picker by examining the number of
/// iterations it would take for every peer in a swarm to obtain a torrent.
/// The rules are described by the TestCfg. Some number of peers are created with
/// a theoretical torrent with some number of pieces.
/// One of these peers will be given the complete download, and all others will start
/// with nothing. We assume every peer uploads at the same rate and will upload to
/// unchoke_limit number fo peers.
/// We simulate the pickers via ticks.
/// Every tick a peer will do these things in this order:
/// Fulfill a single request in its queue
/// The peer whose request was fulfilled will broadcast this to all connected peers
/// Make any number of new requests to other peers
///
/// A general effiency benchmark can then be obtained by counting ticks
/// needed for every peer to complete the torrent.
fn test_efficiency(cfg: TestCfg, picker: Picker) {
    for _ in 0..10 {
        let mut s = Simulation::new(cfg.clone(), picker.clone());
        s.init();
        let t = s.run();
        println!("took {:?} ticks!", t);
        assert!((t as u32) < (((cfg.pieces + cfg.peers as u32) as f32 * 1.5) as u32));
    }
}

#[ignore]
#[test]
fn test_seq_efficiency() {
    let cfg = TestCfg {
        pieces: 40,
        peers: 10,
        unchoke_limit: 5,
        connect_limit: 10,
    };
    let info = Info {
        name: String::from(""),
        announce: String::from(""),
        piece_len: 16384,
        total_len: 16384 * cfg.pieces as u64,
        hashes: vec![vec![0u8]; cfg.pieces as usize],
        hash: [0u8; 20],
        files: vec![],
    };
    let p = Picker::new_sequential(&info);
    test_efficiency(cfg, p);
}

#[test]
fn test_rarest_efficiency() {
    let cfg = TestCfg {
        pieces: 40,
        peers: 10,
        unchoke_limit: 5,
        connect_limit: 10,
    };
    let info = Info {
        name: String::from(""),
        announce: String::from(""),
        piece_len: 16384,
        total_len: 16384 * cfg.pieces as u64,
        hashes: vec![vec![0u8]; cfg.pieces as usize],
        hash: [0u8; 20],
        files: vec![],
    };
    let p = Picker::new_rarest(&info);
    test_efficiency(cfg, p);
}
