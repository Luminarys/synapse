use std::collections::{HashMap, HashSet};
use std::mem;
use std::time;
use torrent::{Info, Peer, Bitfield};
use control::cio;

mod rarest;
mod sequential;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Picker {
    /// Number of blocks per piece
    scale: u64,
    /// Number of detected seeders
    seeders: u16,
    /// Set of pieces which have blocks waiting. These should be prioritized.
    downloading: HashMap<u32, Vec<Downloading>>,
    /// The current picker in use
    picker: PickerKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum PickerKind {
    Rarest(rarest::Picker),
    Sequential(sequential::Picker),
}

#[derive(Clone, Debug, PartialEq)]
struct Downloading {
    offset: u32,
    completed: bool,
    requested: [Option<Request>; 3],
}

struct Request {
    peer: usize,
    requested_at: time::Instant,
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
        let scale = info.piece_len / 16384;
        let picker = rarest::Picker::new(pieces);
        Picker {
            kind: PickerKind::Rarest(picker),
            scale,
            seeders: 0,
            downloading: HashMap::new(),
        }
    }

    /// Creates a new rarest picker, which will select over
    /// the given pieces
    pub fn new_sequential(info: &Info, pieces: &Bitfield) -> Picker {
        let scale = info.piece_len / 16384;
        let picker = sequential::Picker::new(pieces);
        Picker {
            kind: PickerKind::Sequential(picker),
            scale,
            seeders: 0,
            downloading: HashMap::new(),
        }
    }

    pub fn is_sequential(&self) -> bool {
        match &self.kind {
            &PickerKind::Sequential(_) => true,
            _ => false,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        let piece = match self.kind {
            PickerKind::Sequential(ref mut p) => p.pick(peer),
            PickerKind::Rarest(ref mut p) => p.pick(peer),
        };
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, b: Block) -> (bool, Iterator<Item=usize>) {
        // match *self {
        //     Picker::Sequential(ref mut p) => p.completed(idx, offset),
        //     Picker::Rarest(ref mut p) => p.completed(idx, offset),
        // }
    }

    pub fn invalidate_piece(&mut self, idx: u32) {
        match *self {
            Picker::Sequential(ref mut p) => p.incomplete(idx),
            Picker::Rarest(ref mut p) => p.incomplete(idx),
        }
    }

    pub fn piece_available(&mut self, idx: u32) {
        if let PickerKind::Rarest(ref mut p) = *self.kind {
            p.piece_available(idx);
        }
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let PickerKind::Rarest(ref mut p) = *self.kind {
            p.add_peer(peer);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let Picker::Rarest(ref mut p) = *self.kind {
            p.remove_peer(peer);
        }
    }

    pub fn change_picker(&mut self, mut picker: Picker) {
        // mem::swap(self.common(), picker.common());
        // mem::swap(self, &mut picker);
    }

    pub fn unset_waiting(&mut self) {
        // self.common().unset_waiting();
    }
}

impl Block {
    pub fn new(index: u32, offset: u32) -> Block {
        Block { index, offset }
    }
}

/*
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
*/
