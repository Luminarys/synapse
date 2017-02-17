use std::collections::HashMap;
use std::mem;
use piece_field::PieceField;
use peer::Peer;
use torrent::TorrentInfo;

// Implementation based off of: http://blog.libtorrent.org/2011/11/writing-a-fast-piece-picker/

pub struct Picker {
}

impl Picker {
    pub fn new() -> Picker {
        Picker {
        }
    }

    pub fn pick(&mut self, pieces: &PieceField) -> Option<(u32, u32)> {
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
