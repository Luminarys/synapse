use std::collections::HashMap;
use std::mem;
use piece_field::PieceField;
use peer::Peer;
use torrent::TorrentInfo;

pub struct Picker {
    idx: u32,
}

impl Picker {
    pub fn new() -> Picker {
        Picker {
            idx: 0,
        }
    }

    pub fn pick(&mut self, pieces: &PieceField) -> Option<(u32, u32)> {
        if self.idx == pieces.len() {
            return None;
        } else {
            self.idx += 1;
        }
        for i in 0..pieces.len() {
            if !pieces.has_piece(i) {
                return Some((i,0));
            }
        }
        None
    }

    pub fn peer_has_piece(&mut self, piece: u32) {
    }

    pub fn peer_joined(&mut self, pieces: &PieceField) {
    
    }

    pub fn peer_left(&mut self, pieces: &PieceField) {
    
    }
}

#[test]
fn test_default_pick() {
    let mut p = Picker::new();
    let pf = PieceField::new(10);
    assert_eq!(p.pick(&pf), Some((0, 0)));
}
