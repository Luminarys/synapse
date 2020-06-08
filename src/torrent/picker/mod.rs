use std::collections::HashMap;
use std::sync::Arc;
use std::time;

use crate::control::cio;
use crate::torrent::{Bitfield, Info, Peer};
use crate::util::FHashSet;

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
    /// Currently active requests
    downloading: HashMap<Block, Request>,
    /// Blocks requested/completed per piece picked
    blocks: Vec<(usize, usize)>,
    /// Pieces which we've picked fully, but ended up not being downloaded due to a slow peer
    stalled: FHashSet<Block>,
    /// Bitfield of unpicked pieces, not in progress or
    /// completed yet. A set bit is picked, unset is unpicked.
    unpicked: Bitfield,
    /// The current picker in use
    picker: PickerKind,
    /// Piece priorities
    priorities: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    rank: usize,
    requested_at: time::Instant,
    reqd_from: [usize; MAX_DUP_REQS],
    num_reqd: usize,
}

const MAX_DUP_REQS: usize = 3;
const MAX_PC_SIZE: usize = 50;
const MAX_DL_REREQ: usize = 150;
const REQ_TIMEOUT: u64 = 10;

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
        let downloading = if pieces.complete() {
            HashMap::with_capacity(0)
        } else {
            HashMap::with_capacity(8192)
        };
        let blocks = if pieces.complete() {
            Vec::with_capacity(0)
        } else {
            vec![(0, 0); info.pieces() as usize]
        };
        let mut picker = Picker {
            picker: PickerKind::Rarest(picker),
            scale,
            last_piece,
            last_piece_scale,
            downloading,
            seeders: 0,
            unpicked: pieces.clone(),
            stalled: FHashSet::default(),
            priorities: vec![3; info.pieces() as usize],
            blocks,
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

    pub fn done(&mut self) {
        self.downloading = HashMap::with_capacity(0);
        self.blocks = vec![];
        self.stalled = FHashSet::default();
    }

    pub fn tick(&mut self) {
        let mut expired = 0;
        for (block, req) in &mut self.downloading {
            let reqd = self.blocks[block.index as usize].0;
            let _fully_reqd = reqd == self.scale as usize
                || (block.index == self.last_piece && reqd == self.last_piece_scale as usize);
            let deadline = (REQ_TIMEOUT as isize
                + (3 - self.priorities[block.index as usize] as isize))
                as u64;
            if req.requested_at.elapsed().as_secs() >= deadline && !self.stalled.contains(block) {
                expired += 1;
                self.stalled.insert(*block);
            }
        }
        if expired != 0 {
            debug!("Expired {} chunks!", expired);
        }
        if !self.downloading.is_empty() {
            debug!(
                "Unpicked: {}/{}, Downloading: {}",
                self.unpicked.iter().count(),
                self.unpicked.len(),
                self.downloading.len()
            );
        }
    }

    /// Attempts to select a block for a peer.
    pub fn pick<T: cio::CIO>(&mut self, peer: &mut Peer<T>) -> Option<Block> {
        if !self.stalled.is_empty() {
            let block = self.stalled.iter().cloned().find(|b| {
                peer.pieces().has_bit(u64::from(b.index))
                    && !self.downloading[b].has_peer(peer.id())
            });
            if let Some(b) = block {
                self.stalled.remove(&b);
                if let Some(req) = self.downloading.get_mut(&b) {
                    req.force_rereq(peer.id(), peer.rank);
                }
                return Some(b);
            }
        }

        let piece = match self.picker {
            PickerKind::Sequential(ref mut p) => p.pick(peer),
            PickerKind::Rarest(ref mut p) => p.pick(peer),
        };
        piece
            .map(|p| self.pick_piece(p, peer.id(), peer.rank))
            .or_else(|| self.pick_dl(peer))
    }

    /// Picks a block from a given piece for a peer
    fn pick_piece(&mut self, piece: u32, id: usize, rank: usize) -> Block {
        self.blocks[piece as usize].0 += 1;
        let amnt = self.blocks[piece as usize].0;
        let offset = (amnt - 1) as u32 * 16_384;
        if amnt == self.scale as usize
            || (piece == self.last_piece && amnt == self.last_piece_scale as usize)
        {
            match self.picker {
                PickerKind::Sequential(ref mut p) => p.completed(piece),
                PickerKind::Rarest(ref mut p) => p.completed(piece),
            }
            self.unpicked.set_bit(u64::from(piece));
        }
        let block = Block {
            index: piece,
            offset,
        };
        self.downloading.insert(block, Request::new(id, rank));
        block
    }

    /// Attempts to pick the highest priority piece in the dl q
    fn pick_dl<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<Block> {
        let mut dl: Vec<_> = self
            .downloading
            .iter_mut()
            .filter(|&(_, ref req)| req.num_reqd < MAX_DUP_REQS && !req.has_peer(peer.id()))
            .take(MAX_DL_REREQ)
            .collect();
        dl.sort_by_key(|&(_, ref req)| req.num_reqd);
        for (block, req) in dl {
            req.rereq(peer.id(), peer.rank);
            return Some(*block);
        }
        None
    }

    /// Marks a block as completed. Returns a result indicating if the block
    /// was actually requested, the success value containing a bool indicating
    /// if the block is complete.
    pub fn completed<F: FnMut(usize)>(&mut self, b: Block, mut cancel: F) -> Result<bool, ()> {
        self.stalled.remove(&b);
        let dl = self.downloading.remove(&b);
        let dl = match dl {
            Some(dl) => dl,
            None => return Err(()),
        };
        for peer in dl.reqd_from.iter() {
            cancel(*peer);
        }

        self.blocks[b.index as usize].1 += 1;
        let amnt = self.blocks[b.index as usize].1;
        if amnt == self.scale as usize
            || (b.index == self.last_piece && amnt == self.last_piece_scale as usize)
        {
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn have_block(&mut self, b: Block) -> bool {
        !self.downloading.contains_key(&b)
    }

    /// Invalidates a piece
    pub fn invalidate_piece(&mut self, idx: u32) {
        match self.picker {
            PickerKind::Sequential(ref mut p) => p.incomplete(idx),
            PickerKind::Rarest(ref mut p) => p.incomplete(idx),
        }
        if self.blocks.is_empty() {
            self.blocks = vec![(0, 0); self.priorities.len()];
        }
        self.blocks[idx as usize] = (0, 0);
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
        } else if let PickerKind::Rarest(ref mut p) = self.picker {
            p.add_peer(peer);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        // Have to consider situation where a peer became a seeder but joined as leecher.
        if peer.pieces().complete() && self.seeders > 0 {
            self.seeders -= 1;
        } else if let PickerKind::Rarest(ref mut p) = self.picker {
            p.remove_peer(peer);
        }

        for (_, req) in self.downloading.iter_mut() {
            if let Some((idx, _)) = req
                .reqd_from
                .iter()
                .enumerate()
                .find(|&(_, id)| *id == peer.id())
            {
                req.num_reqd -= 1;
                req.reqd_from[idx] = req.reqd_from[req.num_reqd];
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
    }

    pub fn set_priorities(&mut self, pri: &[u8], info: &Arc<Info>) {
        self.unapply_priorities();
        self.priorities = generate_piece_pri(pri, info);
        self.apply_priorities();
    }

    pub fn apply_priorities(&mut self) {
        if self.is_sequential() {
            self.picker = PickerKind::Sequential(sequential::Picker::with_pri(
                &self.unpicked,
                &self.priorities,
            ));
        } else {
            for (piece, pri) in self.priorities.iter().enumerate() {
                if let PickerKind::Rarest(ref mut p) = self.picker {
                    for _ in 0..*pri {
                        p.piece_unavailable(piece as u32);
                    }
                }

                if *pri == 0 && !self.unpicked.has_bit(piece as u64) {
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
                if let PickerKind::Rarest(ref mut p) = self.picker {
                    for _ in 0..*pri {
                        p.piece_available(piece as u32);
                    }
                }

                if *pri == 0 && !self.unpicked.has_bit(piece as u64) {
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
    fn new(peer: usize, rank: usize) -> Request {
        let mut reqd_from = [0; MAX_DUP_REQS];
        reqd_from[0] = peer;
        Request {
            rank,
            requested_at: time::Instant::now(),
            reqd_from,
            num_reqd: 1,
        }
    }

    fn rereq(&mut self, peer: usize, rank: usize) {
        self.rank = rank;
        self.reqd_from[self.num_reqd] = peer;
        self.num_reqd += 1;
        self.requested_at = time::Instant::now();
    }

    fn force_rereq(&mut self, peer: usize, rank: usize) {
        if self.num_reqd < MAX_DUP_REQS {
            self.rereq(peer, rank);
        } else {
            self.reqd_from[0] = peer;
            self.requested_at = time::Instant::now();
            self.rank = rank;
        }
    }

    fn has_peer(&self, peer: usize) -> bool {
        self.reqd_from.contains(&peer)
    }
}
