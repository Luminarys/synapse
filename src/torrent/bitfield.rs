// Use u64 than usize because it conforms with bittorents network protocol
// (4 byte big endian integers)
#[derive(Serialize, Deserialize, Clone)]
pub struct Bitfield {
    len: u64,
    data: Box<[u8]>,
}

impl Bitfield {
    pub fn new(len: u64) -> Bitfield {
        let mut size = len / 8;
        if len % 8 != 0 {
            size += 1;
        }

        Bitfield {
            len: len,
            data: vec![0; size as usize].into_boxed_slice(),
        }
    }

    pub fn from(b: Box<[u8]>, len: u64) -> Bitfield {
        Bitfield {
            len: len,
            data: b,
        }
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn cap(&mut self, len: u64) {
        self.len = len;
    }

    pub fn bytes(&self) -> usize {
        self.data.len()
    }

    pub fn byte_at(&self, pos: u64) -> u8 {
        self.data[pos as usize]
    }

    pub fn complete(&self) -> bool {
        for i in 0..self.data.len() - 1 {
            if !(self.data[i]) != 0 {
                return false;
            }
        }
        if self.len % 8 == 0 {
            return !self.data.last().unwrap() == 0;
        }
        for i in 0..(self.len % 8) {
            if !self.has_bit(self.len - i - 1) {
                return false;
            }
        }
        true
    }

    pub fn has_bit(&self, pos: u64) -> bool {
        debug_assert!(pos < self.len);
        if pos >= self.len {
            false
        } else {
            let block_pos = pos / 8;
            let index = 7 - (pos % 8);
            let block = self.data[block_pos as usize];
            ((block >> index) & 1) == 1
        }
    }

    pub fn set_bit(&mut self, pos: u64) {
        debug_assert!(pos < self.len);
        if pos < self.len {
            let block_pos = pos / 8;
            let index = 7 - (pos % 8);
            let block = self.data[block_pos as usize];
            self.data[block_pos as usize] = block | (1 << index);
        }
    }

    pub fn unset_bit(&mut self, pos: u64) {
        debug_assert!(pos < self.len);
        if pos < self.len {
            let block_pos = pos / 8;
            let index = 7 - (pos % 8);
            let block = self.data[block_pos as usize];
            self.data[block_pos as usize] = block & !(1 << index);
        }
    }

    pub fn usable(&self, other: &Bitfield) -> bool {
        debug_assert!(self.len <= other.len);
        if self.len <= other.len {
            for i in 0..self.data.len() {
                // If we encounter a 0 for us and a 1 for them, return true.
                // XOR will make sure that 0/0 and 1/1 are invalid, and the & with self ensures
                // that only fields which are set on the other bitfield are the 1 in the 1/0 pair.
                if ((self.data[i] ^ other.data[i]) & other.data[i]) > 0 {
                    return true;
                }
            }
        }
        false
    }

    pub fn iter(&self) -> BitfieldIter {
        BitfieldIter::new(self)
    }

    pub fn iter_from(&self, idx: u64) -> BitfieldIter {
        BitfieldIter::from_pos(self, idx)
    }
}

use std::fmt;

impl fmt::Debug for Bitfield {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Bitfield {{ len: {}, pieces: ", self.len)?;
        for i in 0..self.len {
            if self.has_bit(i) {
                write!(f, "1")?;
            } else {
                write!(f, "0")?;
            }
        }
        write!(f, " }}")?;
        Ok(())
    }
}

pub struct BitfieldIter<'a> {
    pf: &'a Bitfield,
    idx: u64,
}

impl<'a> BitfieldIter<'a> {
    fn new(pf: &'a Bitfield) -> BitfieldIter<'a> {
        BitfieldIter { pf: pf, idx: 0 }
    }

    fn from_pos(pf: &'a Bitfield, idx: u64) -> BitfieldIter<'a> {
        BitfieldIter { pf: pf, idx: idx }
    }
}

impl<'a> Iterator for BitfieldIter<'a> {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        while self.idx < self.pf.len() {
            self.idx += 1;
            if self.pf.has_bit(self.idx - 1) {
                return Some(self.idx - 1);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Bitfield;

    #[test]
    fn test_count() {
        let mut pf = Bitfield::new(100);
        for i in 0..100 {
            pf.set_bit(i);
        }
        assert_eq!(pf.iter().count() as u64, pf.len);
    }

    #[test]
    fn test_create() {
        let pf = Bitfield::new(10);
        assert!(pf.len == 10);
        assert!(pf.data.len() == 2)
    }

    #[test]
    fn test_has() {
        let pf = Bitfield::new(10);
        let res = pf.has_bit(9);
        assert!(res == false);
    }

    #[test]
    fn test_set() {
        let mut pf = Bitfield::new(10);

        let res = pf.has_bit(9);
        assert!(res == false);

        pf.set_bit(9);

        let res = pf.has_bit(9);
        assert!(res == true);
    }

    #[test]
    fn test_usable() {
        let mut pf1 = Bitfield::new(10);
        let mut pf2 = Bitfield::new(10);
        assert!(pf1.usable(&pf2) == false);
        pf2.set_bit(9);
        assert!(pf1.usable(&pf2) == true);
        pf1.set_bit(9);
        assert!(pf1.usable(&pf2) == false);
        pf2.set_bit(5);
        assert!(pf1.usable(&pf2) == true);
    }

    #[test]
    fn test_iter() {
        let mut pf = Bitfield::new(10);
        for i in 4..7 {
            pf.set_bit(i as u64);
        }
        pf.iter()
            .map(|r| {
                assert!(r > 3 && r < 7);
            })
            .collect::<Vec<_>>();
    }
}
