use std::collections::HashMap;
use std::{mem, time};
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
    /// Bitfield of unpicked pieces, not in progress or
    /// completed yet. A set bit is picked, unset is unpicked.
    unpicked: Bitfield,
    /// The current picker in use
    picker: PickerKind,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    pub index: u32,
    pub offset: u32,
}

/// Pickers act solely as piece picking algorithm.
/// They will select the optimal next piece for a peer,
/// and can be told when a piece is complete(or invalid).
#[derive(Clone, Debug)]
enum PickerKind {
    Rarest(rarest::Picker),
    Sequential(sequential::Picker),
}

/// A downloading block and the peers it has been
/// requested from.
#[derive(Clone, Debug)]
struct Downloading {
    offset: u32,
    completed: bool,
    requested: Vec<Request>,
}

/// A request to a peer and the time it was initiated.
#[derive(Clone, Debug)]
struct Request {
    peer: usize,
    requested_at: time::Instant,
}

const MAX_DUP_REQS: usize = 3;

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
            unpicked: pieces.clone(),
            downloading: HashMap::new(),
        }
    }

    /// Creates a new sequential picker, which will select over
    /// the given pieces
    pub fn new_sequential(info: &Info, pieces: &Bitfield) -> Picker {
        let scale = info.piece_len / 16384;
        let picker = sequential::Picker::new(pieces);
        Picker {
            picker: PickerKind::Sequential(picker),
            scale,
            seeders: 0,
            unpicked: pieces.clone(),
            downloading: HashMap::new(),
        }
    }

    /// Returns true if the current picker algorithm is sequential
    pub fn is_sequential(&self) -> bool {
        match self.picker {
            PickerKind::Sequential(_) => true,
            _ => false,
        }
    }

    /// Attempts to select a block for a peer.
    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        if let Some(b) = self.pick_expired(peer) {
            return Some(b);
        }

        let piece = match self.picker {
            PickerKind::Sequential(ref mut p) => p.pick(peer),
            PickerKind::Rarest(ref mut p) => p.pick(peer),
        };
        piece.and_then(|p| self.pick_piece(p, peer.id()))
            .or_else(|| self.pick_downloading(peer))
    }

    /// Attempts to pick an expired block
    fn pick_expired<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        // TODO: Use some form of heuristic here to say "we expect to have
        // downloaded some pieces by X, hit the picker with a tick which checks
        // that, flags shit as invalid, and then does a double request
        None
    }

    /// Picks a block from a given piece for a peer
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
            self.unpicked.set_bit(piece as u64);
        }
        Some(Block {
            index: piece,
            offset,
        })
    }

    /// Attempts to pick an already requested block
    fn pick_downloading<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        for (idx, dl) in self.downloading.iter_mut() {
            if peer.pieces().has_bit(*idx as u64) {
                return dl.iter_mut()
                    .find(|r| !r.completed && r.requested.len() < MAX_DUP_REQS)
                    .map(|r| {
                        r.requested.push(Request::new(peer.id()));
                        Block::new(*idx, r.offset)
                    });
            }
        }
        None
    }

    /// Marks a block as completed. Returns a result indicating if the block
    /// was actually requested, the success value containing a bool indicating
    /// if the block is complete, and a vector of peers from which the block
    /// was requested(for cancellation).
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

    /// Invalidates a piece
    pub fn invalidate_piece(&mut self, idx: u32) {
        match self.picker {
            PickerKind::Sequential(ref mut p) => p.incomplete(idx),
            PickerKind::Rarest(ref mut p) => p.incomplete(idx),
        }
        self.unpicked.unset_bit(idx as u64);
    }

    pub fn piece_available(&mut self, idx: u32) {
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.piece_available(idx);
        }
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if peer.pieces().complete() {
            self.seeders += 1;
        }
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.add_peer(peer);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        if let PickerKind::Rarest(ref mut p) = self.picker {
            p.remove_peer(peer);
        }
    }

    /// Alters the picker to sequential/non sequential. If changing
    /// from sequential to non sequential, peer state will need to be loaded
    /// after this.
    pub fn change_picker(&mut self, sequential: bool) {
        self.picker = if sequential {
            PickerKind::Sequential(sequential::Picker::new(&self.unpicked))
        } else {
            PickerKind::Rarest(rarest::Picker::new(&self.unpicked))
        };
        for (i, _) in self.downloading.iter() {
            match self.picker {
                PickerKind::Rarest(ref mut p) => p.dec_avail(*i),
                _ => { }
            }
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
