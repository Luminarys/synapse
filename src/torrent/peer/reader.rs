use std::io::{self, Read, Cursor};
use std::mem;
use torrent::peer::message::Message;
use torrent::piece_field::PieceField;
use byteorder::{BigEndian, ReadBytesExt};
use util::io_err;

pub struct Reader {
    state: ReadState,
    blocks_read: usize,
    download_speed: f64,
}

enum ReadState {
    Idle,
    ReadingHandshake { data: [u8; 68], idx: u8 },
    ReadingLen { data: [u8; 17], idx: u8 },
    ReadingId { data: [u8; 17], len: u32 },
    ReadingMsg { data: [u8; 17], idx: u8, len: u32 },
    ReadingPiece { prefix: [u8; 17], data: Box<[u8; 16384]>, idx: usize },
    ReadingBitfield { data: Vec<u8>, idx: usize },
}

impl ReadState {
    fn next_state<R: Read>(self, conn: &mut R) -> io::Result<Result<Message, ReadState>> {
        // I don't think this could feasibly stack overflow, but possibility should be considered.
        match self {
            ReadState::ReadingHandshake { mut data, mut idx } => {
                idx += conn.read(&mut data[idx as usize..])? as u8;
                if idx == data.len() as u8 {
                    if &data[1..20] != b"BitTorrent protocol" {
                        // return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid protocol used in handshake"));
                    }
                    let mut rsv = [0; 8];
                    rsv.clone_from_slice(&data[20..28]);
                    let mut hash = [0; 20];
                    hash.clone_from_slice(&data[28..48]);
                    let mut pid = [0; 20];
                    pid.clone_from_slice(&data[48..68]);
                    Ok(Ok(Message::Handshake{ rsv: rsv, hash: hash, id: pid }))
                } else {
                    Ok(Err(ReadState::ReadingHandshake { data: data, idx: idx }))
                }
            }
            ReadState::Idle => {
                let mut data = [0; 17];
                let amnt = conn.read(&mut data[0..4])? as u8;
                if amnt != 4 {
                    Ok(Err(ReadState::ReadingLen { data: data, idx: amnt }))
                } else {
                    ReadState::process_len(data, conn)
                }
            }
            ReadState::ReadingLen { mut data, mut idx } => {
                idx += conn.read(&mut data[(idx as usize)..4])? as u8;
                if idx != 4 {
                    Ok(Err(ReadState::ReadingLen { data: data, idx: idx }))
                } else {
                    ReadState::process_len(data, conn)
                }
            }
            ReadState::ReadingId { mut data, len } => {
                let amnt = conn.read(&mut data[4..5])? as u8;
                if amnt != 1 {
                    Ok(Err(ReadState::ReadingId { data: data, len: len }))
                } else {
                    ReadState::process_id(data, len, conn)
                }
            }
            ReadState::ReadingMsg { mut data, mut idx, len } => {
                idx += conn.read(&mut data[(idx as usize)..(len + 4) as usize])? as u8;
                if idx - 4 == len as u8 {
                    match ReadState::process_message(data, len) {
                        Ok(msg) => Ok(Ok(msg)),
                        Err(e) => Err(e),
                    }
                } else {
                    Ok(Err(ReadState::ReadingMsg { data: data, idx: idx, len: len }))
                }
            }
            ReadState::ReadingPiece { mut prefix, mut data, mut idx } => {
                if idx < 13 {
                    idx += conn.read(&mut prefix[idx as usize..13])?;
                    if idx == 13 {
                        ReadState::ReadingPiece { prefix: prefix, data: data, idx: idx }.next_state(conn)
                    } else {
                        Ok(Err(ReadState::ReadingPiece { prefix: prefix, data: data, idx: idx }))
                    }
                } else {
                    idx += conn.read(&mut data[(idx - 13)..])?;
                    if idx - 13 == data.len() {
                        let idx = (&prefix[5..9]).read_u32::<BigEndian>()?;
                        let beg = (&prefix[9..13]).read_u32::<BigEndian>()?;
                        Ok(Ok(Message::Piece{ index: idx, begin: beg, data: data }))
                    } else {
                        Ok(Err(ReadState::ReadingPiece { prefix: prefix, data: data, idx: idx }))
                    }
                }
            }
            ReadState::ReadingBitfield { mut data, mut idx } => {
                idx += conn.read(&mut data[idx as usize..])?;
                let len = data.len();
                if idx == len {
                    Ok(Ok(Message::Bitfield(PieceField::from(data.into_boxed_slice(), len as u32 * 8))))
                } else {
                    Ok(Err(ReadState::ReadingBitfield { data: data, idx: idx }))
                }
            }
        }
    }

    fn process_len<R: Read>(buf: [u8; 17], conn: &mut R) -> io::Result<Result<Message, ReadState>> {
        let len = (&buf[0..4]).read_u32::<BigEndian>()?;
        if len == 0 {
            Ok(Ok(Message::KeepAlive))
        } else {
            ReadState::ReadingId { data: buf, len: len }.next_state(conn)
        }
    }

    fn process_id<R: Read>(buf: [u8; 17], len: u32, conn: &mut R) -> io::Result<Result<Message, ReadState>> {
        let id = buf[4];
        match id {
            0 => Ok(Ok(Message::Choke)),
            1 => Ok(Ok(Message::Unchoke)),
            2 => Ok(Ok(Message::Interested)),
            3 => Ok(Ok(Message::Uninterested)),
            5 => {
                ReadState::ReadingBitfield { data: vec![0; len as usize - 1], idx: 0 }.next_state(conn)
            },
            7 => {
                if len != 16393 {
                    return io_err("Only piece sizes of 16384 are accepted");
                }
                ReadState::ReadingPiece { prefix: buf, data: Box::new([0u8; 16384]), idx: 5}.next_state(conn)
            }
            4 => {
                if len != 5 {
                    return io_err("Invalid Have message length");
                }
                ReadState::ReadingMsg { data: buf, idx: 5, len: len }.next_state(conn)
            },
            6 | 8 => {
                if len != 13 {
                    return io_err("Invalid Request/Cancel message length");
                }
                ReadState::ReadingMsg { data: buf, idx: 5, len: len }.next_state(conn)
            },
            _ => {
                io_err("Invalid ID provided!")
            }
        }
    }

    fn process_message(buf: [u8; 17], len: u32) -> io::Result<Message> {
        match buf[4] {
            4 => {
                if len != 5 {
                    return io_err("Have message must be of len 5");
                }
                Ok(Message::Have((&buf[5..9]).read_u32::<BigEndian>()?))
            }
            6 => {
                if len != 13 {
                    return io_err("Request message must be of len 13");
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>()?;
                let beg = (&buf[9..13]).read_u32::<BigEndian>()?;
                let len = (&buf[13..17]).read_u32::<BigEndian>()?;
                Ok(Message::Request { index: idx, begin: beg, length: len })
            }
            8 => {
                if len != 13 {
                    return io_err("Cancel message must be of len 13");
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>()?;
                let beg = (&buf[9..13]).read_u32::<BigEndian>()?;
                let len = (&buf[13..17]).read_u32::<BigEndian>()?;
                Ok(Message::Cancel { index: idx, begin: beg, length: len })
            }
            _ => {
                io_err("Invalid message ID")
            }
        }
    }
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            state: ReadState::ReadingHandshake { data: [0u8; 68] , idx: 0 },
            blocks_read: 0,
            download_speed: 0.0,
        }
    }

    /// Attempts to read a single message from the connection
    pub fn readable<R: Read>(&mut self, conn: &mut R) -> io::Result<Option<Message>> {
        let state = mem::replace(&mut self.state, ReadState::Idle);
        match state.next_state(conn)? {
            Ok(msg) => {
                self.state = ReadState::Idle;
                Ok(Some(msg))
            }
            Err(new_state) => {
                self.state = new_state;
                Ok(None)
            }
        }
    }
}

#[test]
fn test_read_keepalive() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 0];
    // Test one shot
    if let Message::KeepAlive = r.readable(&mut &data[..]).unwrap().unwrap() {
    } else {
        unreachable!();
    }

    // Test split up
    if let None = r.readable(&mut &data[0..2]).unwrap() {
    } else {
        unreachable!();
    }
    if let Message::KeepAlive = r.readable(&mut &data[2..4]).unwrap().unwrap() {
    } else {
        unreachable!();
    }
}

#[test]
fn test_read_choke() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 1, 0];
    // Test one shot
    if let Message::Choke = r.readable(&mut &data[..]).unwrap().unwrap() {
    } else {
        unreachable!();
    }

    // Test split up
    if let None = r.readable(&mut &data[0..4]).unwrap() {
    } else {
        unreachable!();
    }
    // Simulate spurious read
    if let None = r.readable(&mut &data[4..4]).unwrap() {
    } else {
        unreachable!();
    }
    if let Message::Choke = r.readable(&mut &data[4..5]).unwrap().unwrap() {
    } else {
        unreachable!();
    }
}

#[test]
fn test_read_unchoke() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 1, 1];
    // Test one shot
    if let Message::Unchoke = r.readable(&mut &data[..]).unwrap().unwrap() {
    } else {
        unreachable!();
    }
}

#[test]
fn test_read_interested() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 1, 2];
    // Test one shot
    if let Message::Interested = r.readable(&mut &data[..]).unwrap().unwrap() {
    } else {
        unreachable!();
    }
}

#[test]
fn test_read_uninterested() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 1, 3];
    // Test one shot
    if let Message::Uninterested = r.readable(&mut &data[..]).unwrap().unwrap() {
    } else {
        unreachable!();
    }
}

#[test]
fn test_read_have() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let data = vec![0u8, 0, 0, 5, 4, 0, 0, 0, 1];
    // Test one shot
    match r.readable(&mut &data[..]).unwrap().unwrap() {
        Message::Have(piece) => {
            if piece != 1 {
                unreachable!();
            }
        }
        _ => {
            unreachable!();
        }
    }

    // Test split up
    if let None = r.readable(&mut &data[0..6]).unwrap() {
    } else {
        unreachable!();
    }
    // Simulate spurious read
    if let None = r.readable(&mut &data[6..6]).unwrap() {
    } else {
        unreachable!();
    }
    match r.readable(&mut &data[6..9]).unwrap().unwrap() {
        Message::Have(piece) => {
            if piece != 1 {
                unreachable!();
            }
        }
        _ => {
            unreachable!();
        }
    }
}

#[test]
fn test_read_bitfield() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let mut data = Cursor::new(vec![0u8, 0, 0, 5, 5, 0xff, 0xff, 0xff, 0xff]);
    // Test one shot
    match r.readable(&mut data).unwrap().unwrap() {
        Message::Bitfield(pf) => {
            for i in 0..32 {
                assert!(pf.has_piece(i as u32));
            }
        }
        _ => {
            unreachable!();
        }
    }
}

#[test]
fn test_read_request() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let mut data = Cursor::new(vec![0u8, 0, 0, 13, 6, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]);
    // Test one shot
    match r.readable(&mut data).unwrap().unwrap() {
        Message::Request { index, begin, length } => {
            assert_eq!(index, 1);
            assert_eq!(begin, 1);
            assert_eq!(length, 1);
        }
        _ => {
            unreachable!();
        }
    }
}

#[test]
fn test_read_piece() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let mut info = Cursor::new(vec![0u8, 0, 0x40, 0x09, 7, 0, 0, 0, 1, 0, 0, 0, 1]);
    let mut data = Cursor::new(vec![1u8; 16384]);
    // Test partial read
    assert!(r.readable(&mut info).unwrap().is_none());
    match r.readable(&mut data).unwrap().unwrap() {
        Message::Piece { index, begin, data } => {
            assert_eq!(index, 1);
            assert_eq!(begin, 1);
            for i in 0..16384 {
                assert_eq!(1, data[i]);
            }
        }
        _ => {
            unreachable!();
        }
    }
}

#[test]
fn test_read_cancel() {
    let mut r = Reader::new();
    r.state = ReadState::Idle;
    let mut data = Cursor::new(vec![0u8, 0, 0, 13, 8, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1]);
    // Test one shot
    match r.readable(&mut data).unwrap().unwrap() {
        Message::Cancel { index, begin, length } => {
            assert_eq!(index, 1);
            assert_eq!(begin, 1);
            assert_eq!(length, 1);
        }
        _ => {
            unreachable!();
        }
    }
}

#[test]
fn test_read_handshake() {
    use ::PEER_ID;
    let mut r = Reader::new();
    let m = Message::Handshake { rsv: [0; 8], hash: [0; 20], id: *PEER_ID };
    let mut data = vec![0; 68];
    m.encode(&mut data[..]);
    // Test one shot
    match r.readable(&mut Cursor::new(data)).unwrap().unwrap() {
        Message::Handshake{ rsv, hash, id } => {
            assert_eq!(rsv, [0; 8]);
            assert_eq!(hash, [0; 20]);
            assert_eq!(id, *PEER_ID);
        }
        _ => {
            unreachable!();
        }
    }
}
