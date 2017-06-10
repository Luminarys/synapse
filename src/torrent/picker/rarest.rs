use std::collections::{HashSet, HashMap};
use torrent::{Bitfield, Info, Peer};

pub struct Picker {
    pieces: Bitfield,
    scale: u64,
    waiting: HashSet<u64>,
    waiting_peers: HashMap<u64, HashSet<usize>>,
    endgame_cnt: u64,
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
        let pieces = Bitfield::new(len as u64);
        Picker {
            pieces,
            scale: scale as u64,
            waiting: HashSet::new(),
            endgame_cnt: len as u64,
            waiting_peers: HashMap::new(),
        }
    }

    pub fn add_peer(&mut self, peer: &Peer) {

    }

    pub fn remove_peer(&mut self, peer: &Peer) {

    }

    pub fn piece_available(&mut self, idx: u32) {
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        unimplemented!();
    }

    pub fn completed(&mut self, mut idx: u32, mut offset: u32) -> (bool, HashSet<usize>) {
        unimplemented!();
    }
}
