use std::collections::{HashSet};
use torrent::{PieceField, Info, Peer};

pub struct Picker {
    pieces: PieceField,
    scale: u32,
    waiting: HashSet<u32>,
}

impl Picker {
    pub fn new(info: &Info) -> Picker {
        let scale = info.piece_len/16384;
        // The n - 1 piece length, since the last one is (usually) shorter.
        let compl_piece_len = scale * (info.pieces() as usize - 1);
        // the nth piece length
        let mut last_piece_len = (info.total_len - info.piece_len * (info.pieces() as usize - 1));
        if last_piece_len % 16384 == 0 {
            last_piece_len /= 16384;
        } else {
            last_piece_len /= 16384;
            last_piece_len += 1;
        }
        let len = compl_piece_len + last_piece_len;
        let pieces = PieceField::new(len as u32);
        Picker {
            pieces,
            scale: scale as u32,
            waiting: HashSet::new(),
        }
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        for idx in peer.pieces.iter() {
            let start = idx * self.scale;
            for i in 0..self.scale {
                // On the last piece check, we won't check the whole range.
                if start + i < self.pieces.len() && !self.pieces.has_piece(start + i) {
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
            if (idx + i < self.pieces.len() && !self.pieces.has_piece(idx + i)) || self.waiting.contains(&(idx + i)) {
                return false;
            }
        }
        true
    }

    pub fn chunks(&self) -> u32 {
        self.pieces.len()
    }
}

#[test]
fn test_piece_size() {
    let info = Info {
        announce: String::from(""),
        piece_len: 262144,
        total_len: 2000000,
        hashes: vec![vec![0u8]; 8],
        hash: [0u8; 20],
        files: vec![],
    };

    let mut picker = Picker::new(&info);
    assert_eq!(picker.scale as usize, info.piece_len/16384);
    assert_eq!(picker.pieces.len(), 123);
}
