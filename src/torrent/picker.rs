use std::collections::{HashMap, HashSet};
use torrent::{PieceField, Info, Peer};

pub struct Picker {
    pieces: PieceField,
    scale: u32,
    waiting: HashSet<u32>,
}

impl Picker {
    pub fn new(info: &Info) -> Picker {
        let scale: u32 = info.piece_len as u32/16384;
        let pieces = PieceField::new((scale * (info.hashes.len() as u32)) as u32);
        Picker {
            pieces,
            scale,
            waiting: HashSet::new(),
        }
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        for idx in peer.pieces.iter() {
            let start = idx * self.scale;
            for i in 0..self.scale {
                if !self.pieces.has_piece(start + i) {
                    self.pieces.set_piece(start + i);
                    self.waiting.insert(start + i);
                    return Some((idx, i * 16384));
                }
            }
        }
        None
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, mut idx: u32, mut offset: u32) -> bool {
        offset /= 16384;
        idx *= self.scale;
        self.waiting.remove(&(idx + offset));
        for i in 0..self.scale {
            if !self.pieces.has_piece(idx + i) {
                return false;
            }
        }
        true
    }
}
