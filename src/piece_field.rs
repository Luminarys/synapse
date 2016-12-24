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

    fn has_piece(&self, pos: usize) -> Option<bool> {
        if pos >= self.len {
            return None;
        } else {
            let block_pos = pos/8;
            let index = pos % 8;
            let block = self.data[block_pos];
            Some(((block >> index) & 1) == 1)
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
}

fn test_create() {
    let pf = PieceField::new(10);
    assert!(pf.len == 10);
    assert!(pf.data.len() == 2)
}

fn test_has() {
    let pf = PieceField::new(10);
    let res = pf.has_piece(9);
    assert!(res.is_some());
    assert!(res.unwrap() == false);
}

fn test_set() {
    let mut pf = PieceField::new(10);

    let res = pf.has_piece(9);
    assert!(res.is_some());
    assert!(res.unwrap() == false);

    pf.set_piece(9);

    let res = pf.has_piece(9);
    assert!(res.is_some());
    assert!(res.unwrap() == true);
}
