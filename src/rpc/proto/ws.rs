use byteorder::{BigEndian, ByteOrder};
use std::io;

// Since we never do large transfers of WS itself this should be
// reasonable
const MAX_MSG_BYTES: u64 = 5 * 1000 * 1000;

#[derive(Debug)]
pub enum Frame {
    Text(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Default)]
pub struct Message {
    pub header: u8,
    pub len: u64,
    pub mask: Option<[u8; 4]>,
    pub data: Vec<u8>,
}

#[derive(Debug, PartialEq)]
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
            data: vec![0u8; 256],
            ..Default::default()
        }
    }

    pub fn close() -> Message {
        Message {
            header: 0x80 | Opcode::Close.code(),
            len: 0,
            mask: None,
            data: Vec::with_capacity(0),
        }
    }

    pub fn text(s: String) -> Message {
        let d = s.into_bytes();
        Message {
            header: 0x80 | Opcode::Text.code(),
            len: d.len() as u64,
            mask: None,
            data: d,
        }
    }

    pub fn binary(d: Vec<u8>) -> Message {
        Message {
            header: 0x80 | Opcode::Binary.code(),
            len: d.len() as u64,
            mask: None,
            data: d,
        }
    }

    pub fn pong(d: Vec<u8>) -> Message {
        Message {
            header: 0x80 | Opcode::Pong.code(),
            len: d.len() as u64,
            mask: None,
            data: d,
        }
    }

    pub fn ping(d: Vec<u8>) -> Message {
        Message {
            header: 0x80 | Opcode::Ping.code(),
            len: d.len() as u64,
            mask: None,
            data: d,
        }
    }

    pub fn allocate(&mut self) -> io::Result<()> {
        if self.len > MAX_MSG_BYTES {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        self.data.resize(self.len as usize, 0u8);
        Ok(())
    }

    pub fn fin(&self) -> bool {
        self.header & 0x80 != 0
    }

    pub fn extensions(&self) -> bool {
        self.header & 0x70 != 0
    }

    pub fn opcode(&self) -> Opcode {
        (self.header & 0x0F).into()
    }

    pub fn masked(&self) -> bool {
        self.mask.is_some()
    }

    pub fn serialize(self) -> Vec<u8> {
        let mut prefix = Vec::new();
        let hb2;
        if self.len < 126 {
            hb2 = self.len as u8;
        } else if self.len <= 65_535 {
            hb2 = 126;
        } else {
            hb2 = 127;
        }
        // Ignore masking, server -> client messages shouldn't be
        prefix.push(self.header);
        prefix.push(hb2);

        if hb2 == 126 {
            let mut buf = [0u8; 2];
            BigEndian::write_u16(&mut buf[..], self.len as u16);
            prefix.extend(buf.iter());
        } else if hb2 == 127 {
            let mut buf = [0u8; 8];
            BigEndian::write_u64(&mut buf[..], self.len);
            prefix.extend(buf.iter());
        }
        prefix.extend(self.data.iter());

        prefix
    }
}

impl Into<Message> for Frame {
    fn into(self) -> Message {
        match self {
            Frame::Text(t) => Message::text(t),
            Frame::Binary(b) => Message::binary(b),
        }
    }
}

impl Opcode {
    pub fn is_control(&self) -> bool {
        match *self {
            Opcode::Continuation | Opcode::Text | Opcode::Binary | Opcode::Other(_) => false,

            Opcode::Close | Opcode::Ping | Opcode::Pong | Opcode::OtherControl(_) => true,
        }
    }

    pub fn is_other(&self) -> bool {
        match *self {
            Opcode::OtherControl(_) | Opcode::Other(_) => true,
            _ => false,
        }
    }

    pub fn code(&self) -> u8 {
        match *self {
            Opcode::Continuation => 0,
            Opcode::Text => 1,
            Opcode::Binary => 2,
            Opcode::Other(o) | Opcode::OtherControl(o) => o,
            Opcode::Close => 8,
            Opcode::Ping => 9,
            Opcode::Pong => 10,
        }
    }
}

impl From<u8> for Opcode {
    fn from(b: u8) -> Opcode {
        match b {
            0 => Opcode::Continuation,
            1 => Opcode::Text,
            2 => Opcode::Binary,
            o @ 3..=7 => Opcode::Other(o),
            8 => Opcode::Close,
            9 => Opcode::Ping,
            10 => Opcode::Pong,
            o => Opcode::OtherControl(o),
        }
    }
}
