use byteorder::{BigEndian, WriteBytesExt};

#[derive(Default)]
pub struct Message {
    pub header: u8,
    pub len: u64,
    pub mask: Option<[u8; 4]>,
    pub data: Vec<u8>,
}

pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    OtherControl(u8),
    Other(u8),
}

impl Message {
    pub fn new() -> Message {
        Message {
            data: vec![0u8; 1024],
            ..Default::default()
        }
    }

    pub fn allocate(&mut self) {
        let cl = self.data.len();
        let len = self.len as usize;
        if len > cl {
            self.data.reserve_exact(len - cl);
            unsafe { self.data.set_len(len); }
        }
    }

    pub fn fin(&self) -> bool {
        self.header & 0x80 != 0
    }

    pub fn extensions(&self) -> bool {
        self.header & 0x70 == 0
    }

    pub fn opcode(&self) -> Opcode {
        match self.header & 0x0F {
            0 => Opcode::Continuation,
            1 => Opcode::Text,
            2 => Opcode::Binary,
            o @ 3...7 => Opcode::Other(o),
            8 => Opcode::Close,
            9 => Opcode::Ping,
            10 => Opcode::Pong,
            o => Opcode::OtherControl(o),
        }
    }

    pub fn masked(&self) -> bool {
        self.mask.is_some()
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn mask(&self) -> Option<[u8; 4]> {
        self.mask
    }

    pub fn serialize(mut self) -> Vec<u8> {
        let mut prefix = Vec::new();
        let hb2;
        if self.len < 126 {
            hb2 = self.len as u8;
        } else if self.len < 65535 {
            hb2 = 126;
        } else {
            hb2 = 127;
        }
        // Ignore masking, server -> client messages shouldn't be
        prefix.push(self.header);
        prefix.push(hb2);

        if hb2 == 126 {
            let mut buf = [0u8; 2];
            (&mut buf[..]).write_u16::<BigEndian>(self.len as u16).unwrap();
            prefix.extend(buf.iter());
        } else if hb2 == 127 {
            let mut buf = [0u8; 8];
            (&mut buf[..]).write_u64::<BigEndian>(self.len).unwrap();
            prefix.extend(buf.iter());
        }
        prefix.extend(self.data.iter());

        prefix
    }
}
