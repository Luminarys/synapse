use std::io::{self, Read};
use std::mem;
use message::Message;
use piece_field::PieceField;
use byteorder::{BigEndian, ReadBytesExt};

pub struct Reader {
    state: ReadState,
    blocks_read: usize,
    download_speed: f64,
    received_bitfield: bool,
}

enum ReadState {
    Idle,
    ReadingHandshake { data: [u8; 68], idx: u8 },
    ReadingLen { data: [u8; 17], idx: u8 },
    ReadingId { data: [u8; 17], len: u32 },
    ReadingMsg { data: [u8; 17], idx: u8, len: u32 },
    ReadingPiece { prefix: [u8; 17], data: [u8; 16384], idx: usize },
    ReadingBitfield { data: Vec<u8>, idx: usize },
}

impl ReadState {
    fn next_state<R: Read>(self, conn: &mut R) -> io::Result<Result<Message, ReadState>> {
        // I don't think this could feasibly stack overflow, but possibility should be considered.
        match self {
            ReadState::ReadingHandshake { mut data, mut idx } => {
                idx += conn.read(&mut data[idx as usize..])? as u8;
                if idx == data.len() as u8 - 1 {
                    if &data[1..20] != b"BitTorrent protocol" {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid protocol used in handshake"));
                    }
                    let mut reserved = [0; 8];
                    reserved.clone_from_slice(&data[20..28]);
                    let mut hash = [0; 20];
                    hash.clone_from_slice(&data[28..48]);
                    let mut pid = [0; 20];
                    pid.clone_from_slice(&data[48..68]);
                    Ok(Ok(Message::Handshake(reserved, hash, pid)))
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
                        Ok(Ok(Message::Piece(idx, beg, data)))
                    } else {
                        Ok(Err(ReadState::ReadingPiece { prefix: prefix, data: data, idx: idx }))
                    }
                }
            }
            ReadState::ReadingBitfield { mut data, mut idx } => {
                idx += conn.read(&mut data[idx as usize..])?;
                let len = data.len();
                if idx == len - 1 {
                    Ok(Ok(Message::Bitfield(PieceField::from(data.into_boxed_slice(), len as u32))))
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
                ReadState::ReadingBitfield { data: Vec::with_capacity(len as usize), idx: 0 }.next_state(conn)
            },
            7 => {
                if len != 16393 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Only piece sizes of 16384 are accepted"));
                }
                ReadState::ReadingPiece { prefix: buf, data: [0u8; 16384], idx: len as usize}.next_state(conn)
            }
            _ => {
                ReadState::ReadingMsg { data: buf, idx: 5, len: len }.next_state(conn)
            },
        }
    }

    fn process_message(buf: [u8; 17], len: u32) -> io::Result<Message> {
        match buf[5] {
            4 => {
                if len != 5 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Have message must be of len 5"));
                }
                Ok(Message::Have((&buf[5..9]).read_u32::<BigEndian>()?))
            }
            6 => {
                if len != 13 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Request message must be of len 13"));
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>()?;
                let beg = (&buf[9..13]).read_u32::<BigEndian>()?;
                let len = (&buf[13..17]).read_u32::<BigEndian>()?;
                Ok(Message::Request(idx, beg, len))
            }
            8 => {
                if len != 13 {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "Cancel message must be of len 13"));
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>()?;
                let beg = (&buf[9..13]).read_u32::<BigEndian>()?;
                let len = (&buf[13..17]).read_u32::<BigEndian>()?;
                Ok(Message::Cancel(idx, beg, len))
            }
            _ => {
                Err(io::Error::new(io::ErrorKind::InvalidData, "Invalid message ID"))
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
            received_bitfield: false,
        }
    }

    /// Attempts to read a single message from the connection
    pub fn readable<R: Read>(&mut self, conn: &mut R) -> io::Result<Option<Message>> {
        let state = mem::replace(&mut self.state, ReadState::Idle);
        match state.next_state(conn)? {
            Ok(msg) => {
                if msg.is_bitfield() {
                    if self.received_bitfield {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, "Bitfield cannot be received twice!"));
                    } else {
                        self.received_bitfield = true;
                    }
                }
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
