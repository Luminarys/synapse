use crate::control::cio;
use crate::torrent::{Bitfield, Peer};

#[derive(Clone, Debug)]
pub struct Picker {
    /// The max block index that we've picked up to so far
    piece_idx: usize,
    pieces: Vec<Piece>,
}

#[derive(Clone, Debug)]
struct Piece {
    pos: u32,
    status: PieceStatus,
}

#[derive(Clone, Debug, PartialEq)]
enum PieceStatus {
    Incomplete,
    Complete,
}

impl Picker {
    pub fn new(bf: &Bitfield) -> Picker {
        let mut pieces = [vec![], vec![], vec![], vec![], vec![], vec![]];
        for i in 0..bf.len() {
            if bf.has_bit(i) {
                pieces[0].push(i as u32);
            } else {
                pieces[3].push(i as u32);
            }
        }
        Picker::build(pieces)
    }

    pub fn with_pri(bf: &Bitfield, pri: &[u8]) -> Picker {
        let mut pieces = [vec![], vec![], vec![], vec![], vec![], vec![]];
        for (piece, pri) in pri.iter().enumerate() {
            if bf.has_bit(piece as u64) {
                pieces[0].push(piece as u32);
            } else {
                pieces[*pri as usize].push(piece as u32);
            }
        }
        Picker::build(pieces)
    }

    fn build(pieces: [Vec<u32>; 6]) -> Picker {
        let mut p = vec![];
        for i in &pieces[0] {
            p.push(Piece {
                pos: *i as u32,
                status: PieceStatus::Complete,
            })
        }
        let il = p.len();
        // 5 is highest priority, so start from there
        for i in (1..6).rev() {
            for j in &pieces[i] {
                p.push(Piece {
                    pos: *j as u32,
                    status: PieceStatus::Incomplete,
                })
            }
        }
        Picker {
            piece_idx: il,
            pieces: p,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<u32> {
        self.pieces[self.piece_idx..]
            .iter()
            .find(|p| peer.pieces().has_bit(u64::from(p.pos)))
            .map(|p| p.pos)
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32) {
        if let Some(p) = self.pieces[self.piece_idx..]
            .iter_mut()
            .find(|p| p.pos == idx)
        {
            p.status = PieceStatus::Complete;
        }
        self.update_piece_idx();
    }

    pub fn incomplete(&mut self, idx: u32) {
        let piece_idx = &mut self.piece_idx;
        if let Some((idx, p)) = self
            .pieces
            .iter_mut()
            .enumerate()
            .find(|&(_, ref p)| p.pos == idx)
        {
            p.status = PieceStatus::Incomplete;
            *piece_idx = idx;
        }
    }

    fn update_piece_idx(&mut self) {
        for i in self.piece_idx..self.pieces.len() {
            if self.pieces[i].status == PieceStatus::Complete {
                self.piece_idx += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Picker;
    use crate::torrent::{Bitfield, Peer};

    #[test]
    fn test_piece_pick_order() {
        let b = Bitfield::new(3);
        let mut picker = Picker::new(&b);
        let mut peer = Peer::test_from_pieces(0, b);
        assert_eq!(picker.pick(&peer), None);
        peer.pieces_mut().set_bit(1);
        assert_eq!(picker.pick(&peer), Some(1));
        peer.pieces_mut().set_bit(0);
        assert_eq!(picker.pick(&peer), Some(0));
        picker.completed(0);
        picker.completed(1);
        peer.pieces_mut().set_bit(2);
        assert_eq!(picker.pick(&peer), Some(2));

        picker.completed(2);
        assert_eq!(picker.pick(&peer), None);
        picker.incomplete(1);
        assert_eq!(picker.pick(&peer), Some(1));
    }
}
