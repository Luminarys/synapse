// Use u32 rather than usize because it conforms with bittorents network protocol
// (4 byte big endian integers)
pub struct PieceField {
    len: u32,
    data: Box<[u8]>
}

impl PieceField {
    pub fn new(len: u32) -> PieceField {
        let mut size = len/8;
        if len % 8 != 0 {
            size += 1;
        }

        PieceField {
            len: len,
            data: vec![0; size as usize].into_boxed_slice(),
        }
    }

    pub fn from(b: Box<[u8]>, len: u32) -> PieceField {
        PieceField {
            len: len,
            data: b,
        }
    }

    pub fn extract(self) -> (Box<[u8]>, u32) {
        (self.data, self.len)
    }

    pub fn len(&self) -> u32 {
        self.len
    }

    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    pub fn has_piece(&self, pos: u32) -> bool {
        if pos >= self.len {
            false
        } else {
            let block_pos = pos/8;
            let index = pos % 8;
            let block = self.data[block_pos as usize];
            ((block >> index) & 1) == 1
        }
    }

    pub fn set_piece(&mut self, pos: u32) {
        if pos < self.len {
            let block_pos = pos/8;
            let index = pos % 8;
            let block = self.data[block_pos as usize];
            self.data[block_pos as usize] = block | (1 << index);
        }
    }

    pub fn usable(&self, other: &PieceField) -> bool {
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

use std::fmt;

impl fmt::Debug for PieceField {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "PieceField {{ len: {}, pieces: ", self.len)?;
        for i in 0..self.len {
            if self.has_piece(i) {
                write!(f, "1")?;
            } else {
                write!(f, "0")?;
            }
        }
        write!(f, " }}")?;
        Ok(())
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
    let mut pf1 = PieceField::new(10);
    let mut pf2 = PieceField::new(10);
    assert!(pf1.usable(&pf2) == false);
    pf2.set_piece(9);
    assert!(pf1.usable(&pf2) == true);
    pf1.set_piece(9);
    assert!(pf1.usable(&pf2) == false);
    pf2.set_piece(5);
    assert!(pf1.usable(&pf2) == true);
}
