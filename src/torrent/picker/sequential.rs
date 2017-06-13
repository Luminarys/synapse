use std::collections::{HashSet, HashMap};
use torrent::{Bitfield, Info, Peer};

pub struct Picker {
    /// Number of blocks to pick until endgame
    endgame_cnt: u64,
    /// The max block index that we've picked up to so far
    piece_idx: u64,
    /// Which blocks we've picked
    blocks: Bitfield,
    /// Number of blocks per piece
    scale: u64,
    /// Current blocks we've picked and are waiting for
    waiting: HashSet<u64>,
    /// Peers who we've sent out a block request to
    waiting_peers: HashMap<u64, HashSet<usize>>,
}

impl Picker {
    pub fn new(info: &Info) -> Picker {
        let scale = info.piece_len/16384;
        // The n - 1 piece length, since the last one is (usually) shorter.
        let compl_piece_len = scale * (info.pieces() as usize - 1);
        // the nth piece length
        let mut last_piece_len = info.total_len - info.piece_len as u64 * (info.pieces() as u64 - 1) as u64;
        if last_piece_len % 16384 == 0 {
            last_piece_len /= 16384;
        } else {
            last_piece_len /= 16384;
            last_piece_len += 1;
        }
        let len = compl_piece_len + last_piece_len as usize;
        let blocks = Bitfield::new(len as u64);
        Picker {
            blocks,
            piece_idx: 0,
            scale: scale as u64,
            waiting: HashSet::new(),
            endgame_cnt: len as u64,
            waiting_peers: HashMap::new(),
        }
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        for idx in peer.pieces.iter_from(self.piece_idx) {
            let start = idx * self.scale;
            for i in 0..self.scale {
                // On the last piece check, we won't check the whole range.
                if start + i < self.blocks.len() && !self.blocks.has_bit(start + i) {
                    self.blocks.set_bit(start + i);
                    self.waiting.insert(start + i);
                    let mut hs = HashSet::with_capacity(1);
                    hs.insert(peer.id);
                    self.waiting_peers.insert(start + i, hs);
                    if self.endgame_cnt == 1 {
                        println!("Entering endgame!");
                    }
                    self.endgame_cnt = self.endgame_cnt.saturating_sub(1);
                    return Some((idx as u32, (i * 16384) as u32));
                }
            }
        }
        if self.endgame_cnt == 0 {
            let mut idx = None;
            for piece in self.waiting.iter() {
                if peer.pieces.has_bit(*piece/self.scale) {
                    idx = Some(*piece);
                    break;
                }
            }
            if let Some(i) = idx {
                self.waiting_peers.get_mut(&i).unwrap().insert(peer.id);
                return Some(((i/self.scale) as u32, ((i % self.scale) * 16384) as u32));
            }
        }
        None
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32, offset: u32) -> (bool, HashSet<usize>) {
        let mut idx = idx as u64;
        let mut offset = offset as u64;
        offset /= 16384;
        idx *= self.scale;
        self.waiting.remove(&(idx + offset));
        // TODO: make this less hacky
        let peers = self.waiting_peers.remove(&(idx + offset)).unwrap_or(HashSet::with_capacity(0));
        for i in 0..self.scale {
            if (idx + i < self.blocks.len() && !self.blocks.has_bit(idx + i)) || self.waiting.contains(&(idx + i)) {
                return (false, peers);
            }
        }
        self.update_piece_idx();
        (true, peers)
    }

    fn update_piece_idx(&mut self) {
        let mut idx = self.piece_idx * self.scale;
        loop {
            for i in 0..self.scale {
                if idx + i < self.blocks.len() && !self.blocks.has_bit(idx + i) {
                    return;
                }
            }
            self.piece_idx += 1;
            idx += self.scale;
            if idx > self.blocks.len() {
                return;
            }
        }
    }
}

#[test]
fn test_piece_size() {
    let info = Info {
        name: String::from(""),
        announce: String::from(""),
        piece_len: 262144,
        total_len: 2000000,
        hashes: vec![vec![0u8]; 8],
        hash: [0u8; 20],
        files: vec![],
    };

    let mut picker = Picker::new(&info);
    assert_eq!(picker.scale as usize, info.piece_len/16384);
    assert_eq!(picker.blocks.len(), 123);
}

#[test]
fn test_piece_pick_order() {
    use socket::Socket;
    let info = Info {
        name: String::from(""),
        announce: String::from(""),
        piece_len: 16384,
        total_len: 16384 * 3,
        hashes: vec![vec![0u8]; 3],
        hash: [0u8; 20],
        files: vec![],
    };

    let mut picker = Picker::new(&info);
    let mut peer = Peer::new(Socket::empty());
    peer.pieces = Bitfield::new(4);
    assert_eq!(picker.pick(&peer),None);
    peer.pieces.set_bit(1);
    assert_eq!(picker.pick(&peer), Some((1, 0)));
    peer.pieces.set_bit(0);
    assert_eq!(picker.pick(&peer), Some((0, 0)));
    peer.pieces.set_bit(2);
    assert_eq!(picker.pick(&peer), Some((2, 0)));
}
