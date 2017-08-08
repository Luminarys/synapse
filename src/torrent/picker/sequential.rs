use std::collections::HashSet;
use std::cmp;
use torrent::{Info, Peer, picker};
use control::cio;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Picker {
    /// Common picker data
    pub c: picker::Common,
    /// The max block index that we've picked up to so far
    piece_idx: u64,
}

impl Picker {
    pub fn new(info: &Info) -> Picker {
        Picker {
            c: picker::Common::new(info),
            piece_idx: 0,
        }
    }

    pub fn pick<T: cio::CIO>(&mut self, peer: &Peer<T>) -> Option<(u32, u32)> {
        for idx in peer.pieces().iter_from(self.piece_idx) {
            let start = idx * self.c.scale;
            for i in 0..self.c.scale {
                // On the last piece check, we won't check the whole range.
                if start + i < self.c.blocks.len() && !self.c.blocks.has_bit(start + i) {
                    self.c.blocks.set_bit(start + i);
                    self.c.waiting.insert(start + i);
                    let mut hs = HashSet::with_capacity(1);
                    hs.insert(peer.id());
                    self.c.waiting_peers.insert(start + i, hs);
                    if self.c.endgame_cnt == 1 {
                        println!("Entering endgame!");
                    }
                    self.c.endgame_cnt = self.c.endgame_cnt.saturating_sub(1);
                    return Some((idx as u32, (i * 16384) as u32));
                }
            }
        }
        if self.c.endgame_cnt == 0 {
            let mut idx = None;
            for piece in self.c.waiting.iter() {
                if peer.pieces().has_bit(*piece / self.c.scale) {
                    idx = Some(*piece);
                    break;
                }
            }
            if let Some(i) = idx {
                self.c.waiting_peers.get_mut(&i).unwrap().insert(peer.id());
                return Some((
                    (i / self.c.scale) as u32,
                    ((i % self.c.scale) * 16384) as u32,
                ));
            }
        }
        None
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32, offset: u32) -> (bool, HashSet<usize>) {
        let mut idx = idx as u64;
        let mut offset = offset as u64;
        offset /= 16384;
        idx *= self.c.scale;
        self.c.waiting.remove(&(idx + offset));
        let peers = self.c
            .waiting_peers
            .remove(&(idx + offset))
            .unwrap_or_else(|| HashSet::with_capacity(0));
        for i in 0..self.c.scale {
            if (idx + i < self.c.blocks.len() && !self.c.blocks.has_bit(idx + i)) ||
                self.c.waiting.contains(&(idx + i))
            {
                return (false, peers);
            }
        }
        self.update_piece_idx();
        (true, peers)
    }

    pub fn invalidate_piece(&mut self, idx: u32) {
        self.piece_idx = cmp::min(idx as u64, self.piece_idx);
        self.c.invalidate_piece(idx);
    }

    fn update_piece_idx(&mut self) {
        let mut idx = self.piece_idx * self.c.scale;
        loop {
            for i in 0..self.c.scale {
                if idx + i < self.c.blocks.len() && !self.c.blocks.has_bit(idx + i) {
                    return;
                }
            }
            self.piece_idx += 1;
            idx += self.c.scale;
            if idx > self.c.blocks.len() {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use torrent::{Info, Peer, Bitfield};
    use super::Picker;

    #[test]
    fn test_piece_size() {
        let info = Info {
            name: String::from(""),
            announce: String::from(""),
            piece_len: 262144,
            total_len: 2000000,
            hashes: vec![vec![0u8]; 8],
            hash: [0u8; 20],
            files: vec![],
            private: false,
        };

        let picker = Picker::new(&info);
        assert_eq!(picker.c.scale as u32, info.piece_len / 16384);
        assert_eq!(picker.c.blocks.len(), 123);
    }

    #[test]
    fn test_piece_pick_order() {
        let info = Info::with_pieces(3);

        let mut picker = Picker::new(&info);
        let b = Bitfield::new(4);
        let mut peer = Peer::test_from_pieces(0, b);
        assert_eq!(picker.pick(&peer), None);
        peer.pieces_mut().set_bit(1);
        assert_eq!(picker.pick(&peer), Some((1, 0)));
        peer.pieces_mut().set_bit(0);
        assert_eq!(picker.pick(&peer), Some((0, 0)));
        peer.pieces_mut().set_bit(2);
        assert_eq!(picker.pick(&peer), Some((2, 0)));

        picker.completed(0, 0);
        picker.completed(1, 0);
        picker.completed(2, 0);
        assert_eq!(picker.pick(&peer), None);
        picker.invalidate_piece(1);
        assert_eq!(picker.pick(&peer), Some((1, 0)));
    }
}
