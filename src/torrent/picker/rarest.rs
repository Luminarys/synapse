// Implementation based off of http://blog.libtorrent.org/2011/11/writing-a-fast-piece-picker/

use torrent::{Peer, Bitfield};
use std::ops::IndexMut;
use control::cio;

#[derive(Clone, Debug)]
pub struct Picker {
    /// Current order of pieces
    pieces: Vec<u32>,
    /// Indices into pieces which indicate priority bounds
    priorities: Vec<usize>,
    /// Index mapping a piece to a position in the pieces field
    piece_idx: Vec<PieceInfo>,
}

#[derive(Clone, Debug, PartialEq)]
enum PieceStatus {
    Incomplete,
    Complete,
}

#[derive(Clone, Debug)]
struct PieceInfo {
    idx: usize,
    availability: usize,
    status: PieceStatus,
}

const PIECE_COMPLETE_INC: usize = 100;

impl Picker {
    pub fn new(pieces: &Bitfield) -> Picker {
        let mut piece_idx = Vec::new();
        for i in 0..pieces.len() {
            piece_idx.push(PieceInfo {
                idx: i as usize,
                availability: 0,
                status: PieceStatus::Incomplete,
            });
        }
        let mut p = Picker {
            pieces: (0..pieces.len() as u32).collect(),
            piece_idx,
            priorities: vec![pieces.len() as usize],
        };
        // Start every piece at an availability of 5.
        // This way when we decrement availability for an initial
        // pick we never underflow, and can keep track of which pieces
        // are unpicked(odd) and picked(even).
        for i in (0..pieces.len()).rev() {
            p.piece_available(i as u32);
            if pieces.has_bit(i) {
                p.completed(i as u32);
            }
        }
        p
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        for idx in peer.pieces().iter() {
            self.piece_available(idx as u32);
        }
    }

    pub fn remove_peer<T: cio::CIO>(&mut self, peer: &Peer<T>) {
        for idx in peer.pieces().iter() {
            self.piece_unavailable(idx as u32);
        }
    }

    pub fn piece_available(&mut self, piece: u32) {
        self.inc_avail(piece);
        self.inc_avail(piece);
    }

    pub fn inc_avail(&mut self, piece: u32) {
        let (idx, avail) = {
            let piece = self.piece_idx.index_mut(piece as usize);
            self.priorities[piece.availability] -= 1;
            piece.availability += 1;
            if self.priorities.len() == piece.availability {
                self.priorities.push(self.pieces.len());
            }
            (piece.idx, piece.availability - 1)
        };

        let swap_idx = self.priorities[avail];
        self.swap_piece(idx, swap_idx);
    }

    pub fn piece_unavailable(&mut self, piece: u32) {
        self.dec_avail(piece);
        self.dec_avail(piece);
    }

    pub fn dec_avail(&mut self, piece: u32) {
        let (idx, avail) = {
            let piece = self.piece_idx.index_mut(piece as usize);
            piece.availability -= 1;
            self.priorities[piece.availability] += 1;
            (piece.idx, piece.availability)
        };

        let swap_idx = self.priorities[avail - 1];
        self.swap_piece(idx, swap_idx);
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<u32> {
        // Find the first matching piece which is not complete,
        // and that the peer also has
        self.pieces
            .iter()
            .cloned()
            .filter(|p| {
                self.piece_idx[*p as usize].status == PieceStatus::Incomplete
            })
            .find(|p| peer.pieces().has_bit(u64::from(*p)))
            .map(|p| {
                if (self.piece_idx[p as usize].availability % 2) == 0 {
                    self.dec_avail(p);
                }
                p
            })
    }

    pub fn incomplete(&mut self, piece: u32) {
        self.piece_idx[piece as usize].status = PieceStatus::Incomplete;
        for _ in 0..PIECE_COMPLETE_INC {
            self.piece_unavailable(piece);
        }
    }

    pub fn completed(&mut self, piece: u32) {
        self.piece_idx[piece as usize].status = PieceStatus::Complete;
        // As hacky as this is, it's a good way to ensure that
        // we never waste time picking already selected pieces
        for _ in 0..PIECE_COMPLETE_INC {
            self.piece_available(piece);
        }
    }

    fn swap_piece(&mut self, a: usize, b: usize) {
        self.piece_idx[self.pieces[a] as usize].idx = b;
        self.piece_idx[self.pieces[b] as usize].idx = a;
        self.pieces.swap(a, b);
    }
}

#[cfg(test)]
mod tests {
    use super::Picker;
    use torrent::{Peer, Bitfield};

    #[test]
    fn test_available() {
        let b = Bitfield::new(3);
        let mut picker = Picker::new(&b);
        let mut peers = vec![
            Peer::test_from_pieces(0, b.clone()),
            Peer::test_from_pieces(0, b.clone()),
            Peer::test_from_pieces(0, b.clone()),
        ];
        assert_eq!(picker.pick(&peers[0]), None);

        peers[0].pieces_mut().set_bit(0);
        peers[1].pieces_mut().set_bit(0);
        peers[1].pieces_mut().set_bit(2);
        peers[2].pieces_mut().set_bit(1);

        for peer in peers.iter() {
            picker.add_peer(peer);
        }
        assert_eq!(picker.pick(&peers[1]), Some(2));
        picker.completed(2);
        assert_eq!(picker.pick(&peers[1]), Some(0));
        picker.completed(0);
        assert_eq!(picker.pick(&peers[1]), None);
        assert_eq!(picker.pick(&peers[0]), None);
        assert_eq!(picker.pick(&peers[2]), Some(1));
        picker.completed(1);
    }

    #[test]
    fn test_unavailable() {
        let b = Bitfield::new(3);

        let mut picker = Picker::new(&b);
        let mut peers = vec![
            Peer::test_from_pieces(0, b.clone()),
            Peer::test_from_pieces(0, b.clone()),
            Peer::test_from_pieces(0, b.clone()),
        ];
        assert_eq!(picker.pick(&peers[0]), None);

        peers[0].pieces_mut().set_bit(0);
        peers[0].pieces_mut().set_bit(1);
        peers[1].pieces_mut().set_bit(1);
        peers[1].pieces_mut().set_bit(2);
        peers[2].pieces_mut().set_bit(0);
        peers[2].pieces_mut().set_bit(1);

        for peer in peers.iter() {
            picker.add_peer(peer);
        }
        picker.remove_peer(&peers[0]);

        assert_eq!(picker.pick(&peers[1]), Some(2));
        picker.completed(2);
        assert_eq!(picker.pick(&peers[2]), Some(0));
        picker.completed(0);
        assert_eq!(picker.pick(&peers[2]), Some(1));
        picker.completed(1);

        assert_eq!(picker.pick(&peers[1]), None);
        picker.incomplete(1);
        assert_eq!(picker.pick(&peers[1]), Some(1));
    }
}
