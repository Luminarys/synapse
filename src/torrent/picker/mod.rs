use std::collections::HashMap;
use std::{mem, time};
use std::sync::Arc;
use torrent::{Bitfield, Info, Peer};
use control::cio;

mod rarest;
mod sequential;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug)]
pub struct Picker {
    /// Number of blocks per piece
    scale: u32,
    last_piece_scale: u32,
    last_piece: u32,
    /// Number of detected seeders
    seeders: u16,
    /// Set of pieces which have blocks waiting. These should be prioritized.
    downloading: HashMap<u32, Vec<Downloading>>,
    /// Bitfield of unpicked pieces, not in progress or
    /// completed yet. A set bit is picked, unset is unpicked.
    unpicked: Bitfield,
    /// The current picker in use
    picker: PickerKind,
    /// Piece priorities
    priorities: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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

const MAX_DUP_REQS: usize = 5;
const MAX_DL_Q: usize = 50;
const REQ_TIMEOUT: u64 = 15;

impl Picker {
    /// Creates a new picker, which will select over
    /// the given pieces. The algorithm used for selection
    /// will vary based on the current swarm state, but
    /// will default to rarest first.
    pub fn new(info: &Arc<Info>, pieces: &Bitfield, priorities: &[u8]) -> Picker {
        let scale = info.piece_len / 16_384;
        let picker = rarest::Picker::new(pieces);
        let last_piece = info.pieces().saturating_sub(1);
        let lpl = info.piece_len(last_piece);
        let last_piece_scale = if lpl % 16_384 == 0 {
            lpl / 16_384
        } else {
            lpl / 16_384 + 1
        };
        let mut picker = Picker {
            picker: PickerKind::Rarest(picker),
            scale,
            last_piece,
            last_piece_scale,
            seeders: 0,
            unpicked: pieces.clone(),
            downloading: HashMap::new(),
            priorities: vec![3; info.pieces() as usize],
        };
        picker.set_priorities(priorities, info);
        picker
    }

    /// Returns true if the current picker algorithm is sequential
    pub fn is_sequential(&self) -> bool {
        match self.picker {
            PickerKind::Sequential(_) => true,
            _ => false,
        }
    }

    pub fn tick(&mut self) {
        let mut expired = 0;
        for chunks in self.downloading.values_mut() {
            for chunk in chunks.iter_mut() {
                if !chunk.completed {
                    chunk.requested.retain(|req| {
                        let unexp = req.requested_at.elapsed().as_secs() < REQ_TIMEOUT;
                        if !unexp {
                            expired += 1;
                        }
                        unexp
                    })
                }
            }
        }
        if expired != 0 {
            debug!("Expired {} chunks!", expired);
        }
    }

    /// Attempts to select a block for a peer.
    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        if let Some(b) = self.pick_expired(peer) {
            return Some(b);
        }
        if self.downloading.len() > MAX_DL_Q {
            if let Some(b) = self.pick_pri(peer) {
                return Some(b);
            }
        }

        let piece = match self.picker {
            PickerKind::Sequential(ref mut p) => p.pick(peer),
            PickerKind::Rarest(ref mut p) => p.pick(peer),
        };
        piece
            .and_then(|p| self.pick_piece(p, peer.id()))
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
        self.downloading.entry(piece).or_insert_with(|| vec![]);
        let dl = self.downloading.get_mut(&piece).unwrap();
        let offset = dl.len() as u32 * 16_384;
        dl.push(Downloading {
            offset,
            completed: false,
            requested: vec![Request::new(id)],
        });

        if dl.len() == self.scale as usize
            || (piece == self.last_piece && dl.len() == self.last_piece_scale as usize)
        {
            match self.picker {
                PickerKind::Sequential(ref mut p) => p.completed(piece),
                PickerKind::Rarest(ref mut p) => p.completed(piece),
            }
            self.unpicked.set_bit(u64::from(piece));
        }
        Some(Block {
            index: piece,
            offset,
        })
    }

    /// Attempts to pick the highest priority piece in the dl q
    fn pick_pri<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        let pri = &mut self.priorities;
        self.downloading
            .iter_mut()
            .max_by_key(|&(idx, _)| pri[*idx as usize])
            .and_then(|(idx, dl)| {
                dl.iter_mut()
                    .find(|r| {
                        !r.completed && r.requested.len() < MAX_DUP_REQS
                            && r.requested.iter().all(|req| req.peer != peer.id())
                    })
                    .map(|r| {
                        r.requested.push(Request::new(peer.id()));
                        Block::new(*idx, r.offset)
                    })
            })
    }

    /// Attempts to pick an already requested block
    fn pick_downloading<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        for (idx, dl) in &mut self.downloading {
            if peer.pieces().has_bit(u64::from(*idx)) {
                let r = dl.iter_mut()
                    .find(|r| {
                        !r.completed && r.requested.len() < MAX_DUP_REQS
                            && r.requested.iter().all(|req| req.peer != peer.id())
                    })
                    .map(|r| {
                        r.requested.push(Request::new(peer.id()));
                        Block::new(*idx, r.offset)
                    });
                if r.is_some() {
                    return r;
                }
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
        let res = self.downloading
            .get_mut(&b.index)
            .and_then(|dl| {
                dl.iter_mut()
                    .find(|r| r.offset == b.offset)
                    .map(|r| r.complete())
            })
            .map(|r| r.into_iter().map(|e| e.peer).collect());

        // If we've requested every single block for this piece and they're all complete, remove it
        // and report completion
        let scale = self.scale;
        let lp = self.last_piece;
        let lps = self.last_piece_scale;
        let complete = self.downloading
            .get_mut(&b.index)
            .map(|r| {
                (r.len() as u32 == scale || (b.index == lp && r.len() as u32 == lps))
                    && r.iter().all(|d| d.completed)
            })
            .unwrap_or(false);

        if complete {
            self.downloading.remove(&b.index);
        }

        res.map(|r| (complete, r)).ok_or(())
    }

    pub fn have_block(&mut self, b: Block) -> bool {
        self.downloading
            .get_mut(&b.index)
            .and_then(|dl| {
                dl.iter()
                    .find(|r| r.offset == b.offset)
                    .map(|r| r.completed)
            })
            .unwrap_or(false)
    }

    /// Invalidates a piece
    pub fn invalidate_piece(&mut self, idx: u32) {
        match self.picker {
            PickerKind::Sequential(ref mut p) => p.incomplete(idx),
            PickerKind::Rarest(ref mut p) => p.incomplete(idx),
        }
        self.unpicked.unset_bit(u64::from(idx));
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

        for piece in self.downloading.values_mut() {
            for block in piece {
                block.requested.retain(|req| req.peer != peer.id())
            }
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
        self.apply_priorities();
        for i in self.downloading.keys() {
            if let PickerKind::Rarest(ref mut p) = self.picker {
                p.inc_pri(*i);
            }
        }
    }

    pub fn set_priorities(&mut self, pri: &[u8], info: &Arc<Info>) {
        self.unapply_priorities();
        self.priorities = generate_piece_pri(pri, info);
        self.apply_priorities();
    }

    pub fn apply_priorities(&mut self) {
        if self.is_sequential() {
            let mut pieces = [vec![], vec![], vec![], vec![], vec![], vec![]];
            for (piece, pri) in self.priorities.iter().enumerate() {
                pieces[*pri as usize].push(piece as u32);
            }
            self.picker = PickerKind::Sequential(sequential::Picker::new_pri(pieces));
        } else {
            for (piece, pri) in self.priorities.iter().enumerate() {
                for _ in 0..*pri {
                    if let PickerKind::Rarest(ref mut p) = self.picker {
                        p.piece_unavailable(piece as u32);
                    }
                }

                if *pri == 0 {
                    match self.picker {
                        PickerKind::Rarest(ref mut p) => p.completed(piece as u32),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }

    pub fn unapply_priorities(&mut self) {
        if !self.is_sequential() {
            for (piece, pri) in self.priorities.iter().enumerate() {
                for _ in 0..*pri {
                    if let PickerKind::Rarest(ref mut p) = self.picker {
                        p.piece_available(piece as u32);
                    }
                }

                if *pri == 0 {
                    match self.picker {
                        PickerKind::Rarest(ref mut p) => p.incomplete(piece as u32),
                        _ => unreachable!(),
                    }
                }
            }
        }
    }
}

fn generate_piece_pri(pri: &[u8], info: &Arc<Info>) -> Vec<u8> {
    // Map piece -> priority
    let mut priorities = Vec::with_capacity(info.pieces() as usize);
    // If a piece is completely in a file, just assign that pri.
    // Otherwise mark it as the higher pri piece
    for p in 0..info.pieces() {
        let max = Info::piece_disk_locs(&info, p)
            .map(|loc| pri[loc.file])
            .max()
            .expect("Piece must have locations!");
        priorities.push(max);
    }
    priorities
}

#[cfg(test)]
impl Picker {
    pub fn new_rarest(info: &Info, pieces: &Bitfield) -> Picker {
        Picker::new(
            &Arc::new(info.clone()),
            pieces,
            &vec![3u8; info.files.len()],
        )
    }

    pub fn new_sequential(info: &Info, pieces: &Bitfield) -> Picker {
        let mut p = Picker::new(
            &Arc::new(info.clone()),
            pieces,
            &vec![3u8; info.files.len()],
        );
        p.change_picker(true);
        p
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
