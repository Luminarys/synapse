use std::collections::{HashMap, HashSet};
use std::mem;
use torrent::{Info, Peer, Bitfield};
use control::cio;

mod rarest;
mod sequential;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Picker {
    Rarest(rarest::Picker),
    Sequential(sequential::Picker),
}

/// Common data that all pickers will need
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Common {
    /// Bitfield of which blocks have been waiting
    pub blocks: Bitfield,
    /// Number of blocks per piece
    pub scale: u64,
    /// Set of pieces which have blocks waiting. These should be prioritized.
    pub waiting: HashSet<u64>,
    /// Map of block indeces to peers waiting on them. Used for
    /// cancelling in endgame.
    pub waiting_peers: HashMap<u64, HashSet<usize>>,
    /// Number of blocks left to request. Once this becomes 0
    /// endgame mode is entered.
    pub endgame_cnt: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub index: u32,
    pub offset: u32,
}

impl Picker {
    /// Creates a new rarest picker, which will select over
    /// the given pieces
    pub fn new_rarest(info: &Info, pieces: &Bitfield) -> Picker {
        let picker = rarest::Picker::new(info, pieces);
        Picker::Rarest(picker)
    }

    /// Creates a new rarest picker, which will select over
    /// the given pieces
    pub fn new_sequential(info: &Info, pieces: &Bitfield) -> Picker {
        let picker = sequential::Picker::new(info, pieces);
        Picker::Sequential(picker)
    }

    pub fn is_sequential(&self) -> bool {
        match self {
            &Picker::Sequential(_) => true,
            _ => false,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        match *self {
            Picker::Sequential(ref mut p) => p.pick(peer),
            Picker::Rarest(ref mut p) => p.pick(peer),
        }
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32, offset: u32) -> (bool, HashSet<usize>) {
        match *self {
            Picker::Sequential(ref mut p) => p.completed(idx, offset),
            Picker::Rarest(ref mut p) => p.completed(idx, offset),
        }
    }

    pub fn invalidate_piece(&mut self, idx: u32) {
        match *self {
            Picker::Sequential(ref mut p) => p.invalidate_piece(idx),
            Picker::Rarest(ref mut p) => p.invalidate_piece(idx),
        }
    }

    pub fn piece_available(&mut self, idx: u32) {
        if let Picker::Rarest(ref mut p) = *self {
            p.piece_available(idx);
        }
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let Picker::Rarest(ref mut p) = *self {
            p.add_peer(peer);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let Picker::Rarest(ref mut p) = *self {
            p.remove_peer(peer);
        }
    }

    pub fn change_picker(&mut self, mut picker: Picker) {
        mem::swap(self.common(), picker.common());
        mem::swap(self, &mut picker);
    }

    pub fn unset_waiting(&mut self) {
        self.common().unset_waiting();
    }

    fn common(&mut self) -> &mut Common {
        match *self {
            Picker::Sequential(ref mut p) => &mut p.c,
            Picker::Rarest(ref mut p) => &mut p.c,
        }
    }
}

impl Block {
    pub fn new(index: u32, offset: u32) -> Block {
        Block { index, offset }
    }
}

impl Common {
    pub fn new(info: &Info) -> Common {
        let scale = info.piece_len / 16384;
        // The n - 1 piece length, since the last one is (usually) shorter.
        let compl_piece_len = scale * (info.pieces() - 1);
        // the nth piece length
        let mut last_piece_len = info.total_len -
            info.piece_len as u64 * (info.pieces() as u64 - 1) as u64;
        if last_piece_len % 16384 == 0 {
            last_piece_len /= 16384;
        } else {
            last_piece_len /= 16384;
            last_piece_len += 1;
        }
        let len = compl_piece_len as u64 + last_piece_len;
        let blocks = Bitfield::new(len as u64);
        Common {
            blocks,
            scale: scale as u64,
            waiting: HashSet::new(),
            endgame_cnt: len,
            waiting_peers: HashMap::new(),
        }
    }

    pub fn invalidate_piece(&mut self, idx: u32) {
        let mut unset = false;
        for i in 0..self.scale {
            let bit = idx as u64 * self.scale + i;
            if self.blocks.has_bit(bit) {
                unset = true;
                self.blocks.unset_bit(bit);
            }
        }
        if unset {
            self.endgame_cnt += idx as u64 * self.scale;
        }
    }

    fn unset_waiting(&mut self) {
        for piece in self.waiting.iter() {
            self.blocks.unset_bit(*piece);
        }
        self.endgame_cnt = 0;
        self.waiting.clear();
        self.waiting_peers.clear();
    }
}
