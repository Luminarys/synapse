use std::mem;

pub struct PieceField {
    len: usize,
    data: Box<[u8]>
}

impl PieceField {
    fn new(len: usize) -> PieceField {
        let mut size = len/8;
        if len % 8 != 0 {
            size += 1;
        }

        PieceField {
            len: len,
            data: vec![0; size].into_boxed_slice(),
        }
    }

    fn has_piece(&self, pos: usize) -> bool {
        if pos >= self.len {
            false
        } else {
            let block_pos = pos/8;
            let index = pos % 8;
            let block = self.data[block_pos];
            ((block >> index) & 1) == 1
        }
    }

    fn set_piece(&mut self, pos: usize) {
        if pos < self.len {
            let block_pos = pos/8;
            let index = pos % 8;
            let block = self.data[block_pos];
            self.data[block_pos] = block | (1 << index);
        }
    }

    fn usable(&self, other: &PieceField) -> bool {
        if self.len == other.len {
            for i in 0..self.data.len() {
                // If we encounter a 0 for us and a 1 for them, return true.
                // XOR will make sure that 0/0 and 1/1 are invalid, and the & with self ensures
                // that only fields which are set on the other bitfield are the 1 in the 1/0 pair.
                if ((self.data[i] ^ other.data[i]) & other.data[i]) > 0 {
                    return true;
                }
            }
        }
        return false
    }
}

#[test]
fn test_create() {
    let pf = PieceField::new(10);
    assert!(pf.len == 10);
    assert!(pf.data.len() == 2)
}

#[test]
fn test_has() {
    let pf = PieceField::new(10);
    let res = pf.has_piece(9);
    assert!(res == false);
}

#[test]
fn test_set() {
    let mut pf = PieceField::new(10);

    let res = pf.has_piece(9);
    assert!(res == false);

    pf.set_piece(9);

    let res = pf.has_piece(9);
    assert!(res == true);
}

#[test]
fn test_usable() {
    let pf1 = PieceField::new(10);
    let mut pf2 = PieceField::new(10);
    assert!(pf1.usable(&pf2) == false);
    pf2.set_piece(9);
    assert!(pf1.usable(&pf2) == true);
}
