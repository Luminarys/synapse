use std::fmt;

use crate::protocol;

// Use u64 than usize because it conforms with bittorents network protocol
// (4 byte big endian integers)
#[derive(Clone)]
pub enum Bitfield {
    I { len: u64, data: Box<[u8]>, set: u64 },
    C { len: u64 },
}

impl Bitfield {
    pub fn new(len: u64) -> Bitfield {
        let size = div_round_up!(len, 8);

        Bitfield::I {
            len,
            data: vec![0; size as usize].into_boxed_slice(),
            set: 0,
        }
    }

    pub fn from(b: &[u8], len: u64) -> Bitfield {
        let size = div_round_up!(len, 8);
        let mut vec = b.to_vec();
        vec.resize(size as usize, 0);
        let i = Bitfield::I {
            len,
            data: vec.into_boxed_slice(),
            set: 0,
        };
        let res = Bitfield::I {
            len,
            set: i.iter().count() as u64,
            data: i.into_data(),
        };
        if res.complete() {
            Bitfield::C { len }
        } else {
            res
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
        let size = protocol::Bitfield::bytes(self);
        let mut vec = match self {
            Bitfield::I { data, .. } => data.clone().to_vec(),
            Bitfield::C { .. } => vec![255; size],
        };

        // zero bits beyond len
        let num_bits = self.len() % 8;
        if num_bits > 0 {
            vec[size - 1] &= 0xff << (8 - num_bits);
        }

        vec.into_boxed_slice()
    }

    fn into_data(self) -> Box<[u8]> {
        match self {
            Bitfield::I { data, .. } => data,
            Bitfield::C { len: _ } => {
                let size = protocol::Bitfield::bytes(&self);
                vec![255; size].into_boxed_slice()
            }
        }
    }

    pub fn cap(&mut self, new_len: u64) -> bool {
        // According to the BitTorrent spec, "Clients should drop the
        // connection if they receive bitfields that are not of the
        // correct size, or if the bitfield has any of the spare bits
        // set."
        if new_len > self.len() {
            return false;
        }
        let new_size = div_round_up!(new_len, 8) as usize;
        if new_size != protocol::Bitfield::bytes(self) {
            return false;
        }
        match self {
            Bitfield::I { data, len, .. } => {
                // check for set bits beyond new_len
                let num_bits = new_len % 8;
                if num_bits > 0 && data[new_size - 1] & (0xff >> num_bits) > 0 {
                    return false;
                }
                *len = new_len;
                if self.complete() {
                    *self = Bitfield::C { len: new_len };
                }
            }
            Bitfield::C { len, .. } => {
                if new_len < *len {
                    return false;
                }
            }
        }
        true
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
                    if (block & (1 << index)) == 0 {
                        data[block_pos as usize] = block | (1 << index);
                        *set += 1;
                    }
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
                    if (block & (1 << index)) != 0 {
                        data[block_pos as usize] = block & !(1 << index);
                        *set -= 1;
                    }
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

    pub fn iter(&self) -> BitfieldIter<'_> {
        BitfieldIter::new(self)
    }
}

impl protocol::Bitfield for Bitfield {
    fn bytes(&self) -> usize {
        div_round_up!(self.len(), 8) as usize
    }

    fn byte_at(&self, pos: usize) -> u8 {
        let mut res = match self {
            Bitfield::I { data, .. } => data[pos],
            Bitfield::C { .. } => 255,
        };
        // According to the BitTorrent spec, "Spare bits at the end
        // are set to zero"
        let last_pos = self.bytes() - 1;
        if pos == last_pos {
            // zero bits beyond len
            let num_bits = self.len() - (last_pos as u64) * 8;
            res &= 0xff << (8 - num_bits);
        }
        res
    }
}

impl Default for Bitfield {
    fn default() -> Bitfield {
        Bitfield::new(0)
    }
}

impl fmt::Debug for Bitfield {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> Result<(), fmt::Error> {
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

impl From<Vec<u8>> for Bitfield {
    fn from(data: Vec<u8>) -> Self {
        let len = data.len() as u64 * 8;
        Bitfield::from(&data, len)
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
    use super::protocol;
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
    fn test_data_empty() {
        let pf = Bitfield::new(0);
        assert!(pf.len() == 0);
        assert!(pf.data().len() == 0);
    }

    #[test]
    fn test_data_bits() {
        let indata = vec![0xff; 5];
        let pf = Bitfield::from(&indata, 11);
        let outdata = pf.data();
        assert!(outdata.len() == 2);
        assert!(outdata[0] == 0xff);
        assert!(outdata[1] == 0xe0);
    }

    #[test]
    fn test_has() {
        let pf = Bitfield::new(10);
        let res = pf.has_bit(9);
        assert!(!res);
    }

    #[test]
    fn test_set() {
        let mut pf = Bitfield::new(10);

        assert!(!pf.has_bit(9));
        assert!(pf.set() == 0);

        pf.set_bit(9);

        assert!(pf.has_bit(9));
        assert!(pf.set() == 1);

        pf.set_bit(9); // set it again

        assert!(pf.has_bit(9));
        assert!(pf.set() == 1);
    }

    #[test]
    fn test_set_i() {
        let data = vec![0xff, 0xff, 0x7f];
        let mut bf = Bitfield::from(&data, 21);
        assert_matches!(bf, Bitfield::I { .. });

        bf.set_bit(16);

        assert_matches!(bf, Bitfield::C { len: 21 });
    }

    #[test]
    fn test_unset() {
        let mut pf = Bitfield::new(10);

        assert!(!pf.has_bit(8));
        assert!(!pf.has_bit(9));
        assert!(pf.set() == 0);

        pf.set_bit(9);
        assert!(!pf.has_bit(8));
        assert!(pf.has_bit(9));
        assert!(pf.set() == 1);

        pf.set_bit(8);
        assert!(pf.has_bit(8));
        assert!(pf.has_bit(9));
        assert!(pf.set() == 2);

        pf.unset_bit(9);
        assert!(pf.has_bit(8));
        assert!(!pf.has_bit(9));
        assert!(pf.set() == 1);

        pf.unset_bit(9); // unset it again
        assert!(pf.has_bit(8));
        assert!(!pf.has_bit(9));
        assert!(pf.set() == 1);

        pf.unset_bit(8);
        assert!(!pf.has_bit(8));
        assert!(!pf.has_bit(9));
        assert!(pf.set() == 0);
    }

    #[test]
    fn test_unset_c() {
        let data = vec![0xff, 0xff, 0xff];
        let mut bf = Bitfield::from(&data, 21);
        assert_matches!(bf, Bitfield::C { .. });

        bf.unset_bit(16);

        assert_matches!(bf, Bitfield::I { len: 21, set: 20, .. });
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
        let _ = pf
            .iter()
            .map(|r| {
                assert!(r > 3 && r < 7);
            })
            .collect::<Vec<_>>();
    }

    #[test]
    fn test_c_from() {
        let data = vec![0xff; 2];
        let bf = Bitfield::from(&data, 11);
        assert_matches!(bf, Bitfield::C { .. });
    }

    #[test]
    fn test_byte_at_c() {
        let data = vec![0xff, 0xff, 0xff];
        let bf = Bitfield::from(&data, 21);
        assert_matches!(bf, Bitfield::C { .. });

        assert!(protocol::Bitfield::byte_at(&bf, 2) == 0xf8);
    }

    #[test]
    fn test_byte_at_i() {
        let data = vec![0xff, 0xff, 0x7f];
        let bf = Bitfield::from(&data, 21);
        assert_matches!(bf, Bitfield::I { .. });

        assert!(protocol::Bitfield::byte_at(&bf, 2) == 0x78);
    }

    #[test]
    fn test_cap_c_1() {
        let data = vec![0xff; 5];
        let mut bf = Bitfield::from(&data, 11);
        assert_matches!(bf, Bitfield::C { .. });

        assert!(!bf.cap(15));
    }

    #[test]
    fn test_cap_c_2() {
        let data = vec![0xff; 5];
        let mut bf = Bitfield::from(&data, 11);
        assert_matches!(bf, Bitfield::C { .. });

        assert!(bf.cap(11));
    }

    #[test]
    fn test_cap_c_3() {
        let data = vec![0xff; 5];
        let mut bf = Bitfield::from(&data, 11);
        assert_matches!(bf, Bitfield::C { .. });

        assert!(!bf.cap(10));
    }

    #[test]
    fn test_cap_i_1() {
        let data = vec![0xff, 0xff, 0xf8];
        let mut bf = Bitfield::from(&data, 24);
        assert_matches!(bf, Bitfield::I { .. });

        assert!(bf.cap(21));

        assert_matches!(bf, Bitfield::C { .. });
    }

    #[test]
    fn test_cap_i_2() {
        let data = vec![0xff, 0xff, 0xf8];
        let mut bf = Bitfield::from(&data, 24);
        assert_matches!(bf, Bitfield::I { .. });

        assert!(!bf.cap(16));
    }

    #[test]
    fn test_cap_i_3() {
        let data = vec![0xff, 0xff, 0xf8];
        let mut bf = Bitfield::from(&data, 24);
        assert_matches!(bf, Bitfield::I { .. });

        assert!(!bf.cap(25));
    }
}
