use std::collections::{HashMap, HashSet};
use std::{mem, time, vec};
use torrent::{Info, Peer, Bitfield};
use control::cio;

mod rarest;
mod sequential;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct Picker {
    /// Number of blocks per piece
    scale: u32,
    /// Number of detected seeders
    seeders: u16,
    /// Set of pieces which have blocks waiting. These should be prioritized.
    downloading: HashMap<u32, Vec<Downloading>>,
    /// The current picker in use
    picker: PickerKind,
}

#[derive(Clone, Debug)]
enum PickerKind {
    Rarest(rarest::Picker),
    Sequential(sequential::Picker),
}

#[derive(Clone, Debug)]
struct Downloading {
    offset: u32,
    completed: bool,
    requested: Vec<Request>,
}

#[derive(Clone, Debug)]
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
            picker: PickerKind::Rarest(picker),
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
            picker: PickerKind::Sequential(picker),
            scale,
            seeders: 0,
            downloading: HashMap::new(),
        }
    }

    pub fn is_sequential(&self) -> bool {
        match self.picker {
            PickerKind::Sequential(_) => true,
            _ => false,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        let piece = match self.picker {
            PickerKind::Sequential(ref mut p) => p.pick(peer),
            PickerKind::Rarest(ref mut p) => p.pick(peer),
        };
        piece.and_then(|p| self.pick_piece(p, peer.id()))
            .or_else(|| self.pick_downloading(peer))
    }

    fn pick_piece(&mut self, piece: u32, id: usize) -> Option<Block> {
        if !self.downloading.contains_key(&piece) {
            self.downloading.insert(piece, vec![]);
        }
        let dl = self.downloading.get_mut(&piece).unwrap();
        let offset = dl.len() as u32* 16384;
        dl.push(
            Downloading {
                offset,
                completed: false,
                requested: vec![Request::new(id)],
            });

        if dl.len() == self.scale as usize {
            match self.picker {
                PickerKind::Sequential(ref mut p) => p.completed(piece),
                PickerKind::Rarest(ref mut p) => p.completed(piece),
            }
        }
        Some(Block {
            index: piece,
            offset,
        })
    }

    fn pick_downloading<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        for (idx, dl) in self.downloading.iter_mut() {
            if peer.pieces().has_bit(*idx as u64) {
                return dl.iter_mut()
                    .find(|r| !r.completed)
                    .map(|r| {
                        r.requested.push(Request::new(peer.id()));
                        Block::new(*idx, r.offset)
                    });
            }
        }
        None
    }

    /// Returns whether or not the whole piece is complete.
    /// The error value indicates if the block was invalid(not requested)
    pub fn completed(&mut self, b: Block) -> Result<(bool, Vec<usize>), ()> {
        // Find the block in our downloading blocks, mark as true,
        // and extract the current peer list for return.
        let res = self.downloading.get_mut(&b.index)
            .and_then(|dl| dl.iter_mut()
                             .find(|r| r.offset == b.offset)
                             .map(|r| r.complete()))
            .map(|r| r.into_iter().map(|e| e.peer).collect());
        // If we've requested every single block for this piece and they're all complete, remove it
        // and report completion
        let scale = self.scale;
        let complete = self.downloading.get_mut(&b.index)
            .map(|r| r.len() as u32 == scale && r.iter().all(|d| d.completed)).unwrap_or(false);

        if complete {
            self.downloading.remove(&b.index);
        }

        res.map(|r| (complete, r)).ok_or(())
    }

    pub fn invalidate_piece(&mut self, idx: u32) {
        match self.picker {
            PickerKind::Sequential(ref mut p) => p.incomplete(idx),
            PickerKind::Rarest(ref mut p) => p.incomplete(idx),
        }
    }

    pub fn piece_available(&mut self, idx: u32) {
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.piece_available(idx);
        }
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.add_peer(peer);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.remove_peer(peer);
        }
    }
}

impl Block {
    pub fn new(index: u32, offset: u32) -> Block {
        Block { index, offset }
    }
}

impl Request {
    fn new(peer: usize) -> Request {
        Request {
            peer,
            requested_at: time::Instant::now(),
        }
    }
}

impl Downloading {
    fn complete(&mut self) -> Vec<Request> {
        self.completed = true;
        mem::replace(&mut self.requested, Vec::with_capacity(0))
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
