use torrent::{Bitfield, Peer};
use control::cio;

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
    pub fn new(pieces: &Bitfield) -> Picker {
        let mut p = (0..pieces.len())
            .filter(|p| pieces.has_bit(*p))
            .map(|p| {
                Piece {
                    pos: p as u32,
                    status: PieceStatus::Complete,
                }
            })
            .collect::<Vec<_>>();
        let il = p.len();
        p.extend((0..pieces.len()).filter(|p| !pieces.has_bit(*p)).map(|p| {
            Piece {
                pos: p as u32,
                status: PieceStatus::Incomplete,
            }
        }));

        Picker {
            piece_idx: il,
            pieces: p,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<u32> {
        self.pieces[self.piece_idx..]
            .iter()
            .find(|p| peer.pieces().has_bit(p.pos as u64))
            .map(|p| p.pos)
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32) {
        self.pieces[self.piece_idx..]
            .iter_mut()
            .find(|p| p.pos == idx)
            .map(|p| p.status = PieceStatus::Complete);
        self.update_piece_idx();
    }

    pub fn incomplete(&mut self, idx: u32) {
        let piece_idx = &mut self.piece_idx;
        self.pieces
            .iter_mut()
            .enumerate()
            .find(|&(_, ref p)| p.pos == idx)
            .map(|(idx, p)| {
                p.status = PieceStatus::Incomplete;
                *piece_idx = idx;
            });
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
    use torrent::{Info, Peer, Bitfield};
    use super::Picker;

    #[test]
    fn test_piece_pick_order() {
        let info = Info::with_pieces(3);

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
