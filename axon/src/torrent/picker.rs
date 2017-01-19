use piece_field::PieceField;
use peer::Peer;

// Implementation based off of: http://blog.libtorrent.org/2011/11/writing-a-fast-piece-picker/

pub struct Picker {
}

struct Piece {
    peer_count: usize,
    partial: bool,
    index: usize,
}

impl Picker {
    pub fn new() -> Picker {
        Picker {
        }
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        (0, 0)
    }

    pub fn peer_has_piece(&mut self, peer: &Peer, piece: u32) {
    
    }

    pub fn peer_joined(&mut self, peer: &Peer, piece: u32) {
    
    }

    pub fn peer_left(&mut self, peer: &Peer, piece: u32) {
    
    }
}
