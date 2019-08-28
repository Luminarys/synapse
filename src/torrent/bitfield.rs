// Use u64 than usize because it conforms with bittorents network protocol
// (4 byte big endian integers)
#[derive(Clone)]
pub enum Bitfield {
    I { len: u64, data: Box<[u8]>, set: u64 },
    C { len: u64 },
}

impl Bitfield {
    pub fn new(len: u64) -> Bitfield {
        let mut size = len / 8;
        if len % 8 != 0 {
            size += 1;
        }

        Bitfield::I {
            len,
            data: vec![0; size as usize].into_boxed_slice(),
            set: 0,
        }
    }

    pub fn from(b: Box<[u8]>, len: u64) -> Bitfield {
        let i = Bitfield::I {
            len,
            data: b,
            set: 0,
        };
        let set = i.iter().count() as u64;
        if i.complete() {
            Bitfield::C { len }
        } else {
            Bitfield::I {
                len,
                data: i.into_data(),
                set,
            }
        }
    }

    pub fn len(&self) -> u64 {
        match self {
            Bitfield::I { len, .. } => *len,
            Bitfield::C { len } => *len,
        }
    }

    pub fn set(&self) -> u64 {
        match self {
            Bitfield::I { set, .. } => *set,
            Bitfield::C { len } => *len,
        }
    }

    pub fn data(&self) -> Box<[u8]> {
        match self {
            Bitfield::I { data, .. } => data.clone(),
            Bitfield::C { len } => {
                let mut size = len / 8;
                if len % 8 != 0 {
                    size += 1;
                }
                vec![255; size as usize].into_boxed_slice()
            }
        }
    }

    pub fn into_data(self) -> Box<[u8]> {
        match self {
            Bitfield::I { data, .. } => data,
            Bitfield::C { len } => {
                let mut size = len / 8;
                if len % 8 != 0 {
                    size += 1;
                }
                vec![255; size as usize].into_boxed_slice()
            }
        }
    }

    pub fn cap(&mut self, l: u64) {
        match self {
            Bitfield::I { len, .. } => *len = l,
            Bitfield::C { len } => *len = l,
        }
    }

    pub fn bytes(&self) -> usize {
        let mut size = self.len() / 8;
        if self.len() % 8 != 0 {
            size += 1;
        }
        size as usize
    }

    pub fn byte_at(&self, pos: u64) -> u8 {
        match self {
            Bitfield::I { data, .. } => data[pos as usize],
            Bitfield::C { .. } => 255,
        }
    }

    pub fn complete(&self) -> bool {
        match self {
            Bitfield::I { len, data, set } => {
                // Fail safe for magnets
                if data.len() == 0 {
                    return false;
                }
                set == len
            }
            Bitfield::C { .. } => true,
        }
    }

    pub fn has_bit(&self, pos: u64) -> bool {
        debug_assert!(pos < self.len());
        if pos >= self.len() {
            false
        } else {
            match self {
                Bitfield::I { data, .. } => {
                    let block_pos = pos / 8;
                    let index = 7 - (pos % 8);
                    let block = data[block_pos as usize];
                    ((block >> index) & 1) == 1
                }
                Bitfield::C { .. } => true,
            }
        }
    }

    pub fn set_bit(&mut self, pos: u64) {
        debug_assert!(pos < self.len());
        if pos < self.len() {
            match self {
                Bitfield::I { data, set, .. } => {
                    let block_pos = pos / 8;
                    let index = 7 - (pos % 8);
                    let block = data[block_pos as usize];
                    data[block_pos as usize] = block | (1 << index);
                    *set += 1;
                }
                Bitfield::C { .. } => {}
            }
            if self.complete() {
                *self = Bitfield::C { len: self.len() };
            }
        }
    }

    pub fn unset_bit(&mut self, pos: u64) {
        debug_assert!(pos < self.len());
        if pos < self.len() {
            if let Bitfield::C { .. } = self {
                *self = Bitfield::I {
                    len: self.len(),
                    data: self.data(),
                    set: self.set(),
                };
            }
            match self {
                Bitfield::I { data, set, .. } => {
                    let block_pos = pos / 8;
                    let index = 7 - (pos % 8);
                    let block = data[block_pos as usize];
                    data[block_pos as usize] = block & !(1 << index);
                    *set -= 1;
                }
                Bitfield::C { .. } => unreachable!(),
            }
        }
    }

    pub fn usable(&self, other: &Bitfield) -> bool {
        debug_assert!(self.len() <= other.len());
        if self.len() <= other.len() {
            return match (self, other) {
                (Bitfield::I { data, .. }, Bitfield::I { data: od, .. }) => {
                    for i in 0..data.len() {
                        // If we encounter a 0 for us and a 1 for them, return true.
                        // XOR will make sure that 0/0 and 1/1 are invalid, and the & with self ensures
                        // that only fields which are set on the other bitfield are the 1 in the 1/0 pair.
                        if ((data[i] ^ od[i]) & od[i]) > 0 {
                            return true;
                        }
                    }
                    false
                }
                (Bitfield::I { .. }, Bitfield::C { .. }) => true,
                (Bitfield::C { .. }, _) => false,
            };
        }
        false
    }

    pub fn b64(&self) -> String {
        base64::encode(&self.data())
    }

    pub fn iter(&self) -> BitfieldIter {
        BitfieldIter::new(self)
    }
}

impl Default for Bitfield {
    fn default() -> Bitfield {
        Bitfield::new(0)
    }
}

use std::fmt;

impl fmt::Debug for Bitfield {
    fn fmt(&self, f: &mut fmt::Formatter) -> Result<(), fmt::Error> {
        write!(f, "Bitfield {{ len: {}, pieces: ", self.len())?;
        for i in 0..self.len() {
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
        BitfieldIter { pf, idx: 0 }
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
        assert_eq!(pf.iter().count() as u64, pf.len());
    }

    #[test]
    fn test_create() {
        let pf = Bitfield::new(10);
        assert!(pf.len() == 10);
        assert!(pf.data().len() == 2)
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
