use super::{Block, Picker};
use std::collections::HashMap;
use std::cell::UnsafeCell;
use torrent::{Bitfield, Peer as TGPeer, Info};
use rand::distributions::{IndependentSample, Range};
use {rand, control};

type TPeer = TGPeer<control::cio::test::TCIO>;

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
            let connected =
                rand::sample(&mut rng, 0..cfg.peers as usize, cfg.connect_limit as usize);
            let unchoked = rand::sample(
                &mut rng,
                connected.iter().map(|v| *v),
                cfg.unchoke_limit as usize,
            );
            let peer = Peer {
                picker: picker.clone(),
                connected,
                unchoked,
                unchoked_by: Vec::new(),
                requests: Vec::new(),
                requested_pieces: HashMap::new(),
                compl: None,
                data: {
                    TPeer::test(i as usize, 0, 0, 0, Bitfield::new(cfg.pieces as u64))
                },
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
            self.peers()[0].data.pieces_mut().set_bit(i as u64);
        }
        assert!(self.peers()[0].data.pieces().complete());
        for peer in self.peers().iter() {
            for pid in peer.unchoked.iter() {
                self.peers()[*pid].unchoked_by.push(peer.data.id());
            }
        }
        for peer in self.peers().iter_mut() {
            for pid in 0..self.cfg.peers {
                peer.requested_pieces.insert(pid as usize, 0);
            }
        }
    }

    fn run(&mut self) -> (usize, f64) {
        while let Err(()) = self.tick() {
            self.ticks += 1;
            if self.ticks as u32 >= 3 * (self.cfg.pieces + self.cfg.peers as u32) {
                panic!();
            }
        }
        let mut total = 0.;
        for peer in self.peers().iter().skip(1) {
            total += peer.compl.unwrap() as f64;
        }
        return (self.ticks, total / (self.cfg.peers as f64 - 1.));
    }

    fn tick(&mut self) -> Result<(), ()> {
        let mut rng = rand::thread_rng();
        for peer in self.peers().iter_mut() {
            for _ in 0..self.cfg.req_per_tick {
                if !peer.requests.is_empty() {
                    let req = if true {
                        peer.requests.pop().unwrap()
                    } else {
                        let b = Range::new(0, peer.requests.len());
                        peer.requests.remove(b.ind_sample(&mut rng))
                    };
                    let ref mut received = self.peers()[req.peer];
                    received.picker.completed(Block::new(req.piece, 0)).unwrap();
                    received.data.pieces_mut().set_bit(req.piece as u64);
                    if received.data.pieces().complete() {
                        received.compl = Some(self.ticks);
                        for p in self.peers().iter_mut() {
                            if !p.data.pieces().complete() &&
                                !p.unchoked_by.contains(&peer.data.id())
                            {
                                p.unchoked_by.push(peer.data.id());
                            }
                        }
                    }
                    *received.requested_pieces.get_mut(&peer.data.id()).unwrap() -= 1;
                    for pid in received.connected.iter() {
                        self.peers()[*pid].picker.piece_available(req.piece);
                    }
                }
            }

            for pid in peer.unchoked_by.iter() {
                let ref mut ucp = self.peers()[*pid];
                let cnt = peer.requested_pieces.get_mut(&ucp.data.id()).unwrap();
                if peer.data.pieces().usable(ucp.data.pieces()) {
                    while *cnt < self.cfg.req_queue_len {
                        if let Some(block) = peer.picker.pick(&ucp.data) {
                            ucp.requests.push(Request {
                                peer: peer.data.id(),
                                piece: block.index,
                            });
                            *cnt += 1;
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        let inc = self.peers()
            .iter()
            .filter(|p| !p.data.pieces().complete())
            .map(|p| p.data.id())
            .collect::<Vec<_>>();
        if inc.is_empty() { Ok(()) } else { Err(()) }
    }

    fn peers<'f>(&self) -> &'f mut Vec<Peer> {
        unsafe { self.peers.get().as_mut().unwrap() }
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
    requested_pieces: HashMap<usize, u8>,
    compl: Option<usize>,
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
    req_per_tick: u8,
    req_queue_len: u8,
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
    let mut total = 0;
    let mut pat = 0.;
    let num_runs = 20;
    for _ in 0..num_runs {
        let mut s = Simulation::new(cfg.clone(), picker.clone());
        s.init();
        let (t, a) = s.run();
        total += t;
        pat += a;
    }
    let ta = total / num_runs;
    println!("Avg: {:?}", ta);
    println!("Avg peer ticks: {:?}", pat / num_runs as f64);
    assert!((ta as u32) < (((cfg.pieces + cfg.peers as u32) as f32 * 1.5) as u32));
}

#[ignore]
#[test]
fn test_seq_efficiency() {
    let cfg = TestCfg {
        pieces: 100,
        peers: 20,
        unchoke_limit: 5,
        connect_limit: 20,
        req_per_tick: 2,
        req_queue_len: 2,
    };
    let info = Info::with_pieces(cfg.pieces as usize);
    let b = Bitfield::new(cfg.pieces as u64);
    let p = Picker::new_sequential(&info, &b);
    test_efficiency(cfg, p);
}

#[ignore]
#[test]
fn test_rarest_efficiency() {
    let cfg = TestCfg {
        pieces: 100,
        peers: 20,
        unchoke_limit: 5,
        connect_limit: 20,
        req_per_tick: 2,
        req_queue_len: 2,
    };
    let info = Info::with_pieces(cfg.pieces as usize);
    let b = Bitfield::new(cfg.pieces as u64);
    let p = Picker::new_rarest(&info, &b);
    test_efficiency(cfg, p);
}

#[test]
fn test_seq_picker() {
    let mut i = Info::with_pieces(10);
    i.piece_idx = Info::generate_piece_idx(i.hashes.len(), i.piece_len as u64, &i.files);
    let b = Bitfield::new(10);
    let mut p = Picker::new_sequential(&i, &b);
    let mut pb = Bitfield::new(10);
    for i in 0..10 {
        pb.set_bit(i);
    }
    let peer = TPeer::test_from_pieces(0, pb);

    for i in 0..10 {
        assert_eq!(p.pick(&peer), Some(Block::new(i, 0)));
    }

    for i in 0..10 {
        assert_eq!(p.completed(Block::new(i, 0)), Ok((true, vec![0])));
    }

    p.invalidate_piece(5);

    assert_eq!(p.pick(&peer), Some(Block::new(5, 0)));
}
