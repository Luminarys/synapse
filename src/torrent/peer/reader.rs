use std::io::{self, Read, ErrorKind};
use std::mem;
use torrent::peer::Message;
use torrent::Bitfield;
use byteorder::{BigEndian, ReadBytesExt};
use util::{io_err, io_err_val};

pub(super) struct Reader {
    state: ReadState,
    blocks_read: usize,
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            state: ReadState::ReadingHandshake {
                data: [0u8; 68],
                idx: 0,
            },
            blocks_read: 0,
        }
    }

    /// Attempts to read a message from the connection
    pub fn readable<R: Read>(&mut self, conn: &mut R) -> io::Result<Option<Message>> {
        // Keep on trying to read until we get an EWOULDBLOCK error.
        let state = mem::replace(&mut self.state, ReadState::Idle);
        match state.next_state(conn) {
            ReadRes::Message(msg) => {
                self.state = ReadState::Idle;
                if msg.is_piece() {
                    self.blocks_read += 1
                }
                Ok(Some(msg))
            }
            ReadRes::Incomplete(state) => {
                self.state = state;
                Ok(None)
            }
            ReadRes::EOF => io_err("EOF"),
            ReadRes::Err(e) => Err(e),
        }
    }
}

enum ReadState {
    Idle,
    ReadingHandshake { data: [u8; 68], idx: u8 },
    ReadingLen { data: [u8; 17], idx: u8 },
    ReadingId { data: [u8; 17], len: u32 },
    ReadingMsg { data: [u8; 17], idx: u8, len: u32 },
    ReadingPiece {
        prefix: [u8; 17],
        data: Box<[u8; 16_384]>,
        len: u32,
        idx: usize,
    },
    ReadingBitfield { data: Vec<u8>, idx: usize },
}

enum ReadRes {
    /// A complete message was read
    Message(Message),
    /// WouldBlock error was encountered, cannot
    /// complete read.
    Incomplete(ReadState),
    /// 0 bytes were read, indicating EOF
    EOF,
    /// An unknown IO Error occured.
    Err(io::Error),
}

impl ReadState {
    /// Continuously reads fron conn until a WouldBlock error is received,
    /// a complete message is read, EOF is encountered, or some other
    /// IO error is encountered.
    fn next_state<R: Read>(self, conn: &mut R) -> ReadRes {
        // I don't think this could feasibly stack overflow, but possibility should be considered.
        match self {
            ReadState::ReadingHandshake { mut data, mut idx } => {
                match conn.read(&mut data[idx as usize..]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(amnt) => {
                        idx += amnt as u8;
                        if idx == data.len() as u8 {
                            if &data[1..20] != b"BitTorrent protocol" {
                                return ReadRes::Err(
                                    io_err_val("Invalid protocol used in handshake"),
                                );
                            }
                            let mut rsv = [0; 8];
                            rsv.clone_from_slice(&data[20..28]);
                            let mut hash = [0; 20];
                            hash.clone_from_slice(&data[28..48]);
                            let mut pid = [0; 20];
                            pid.clone_from_slice(&data[48..68]);
                            ReadRes::Message(Message::Handshake {
                                rsv: rsv,
                                hash: hash,
                                id: pid,
                            })
                        } else {
                            ReadState::ReadingHandshake {
                                data: data,
                                idx: idx,
                            }.next_state(conn)
                        }
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::ReadingHandshake {
                            data: data,
                            idx: idx,
                        })
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
            ReadState::Idle => {
                let mut data = [0; 17];
                match conn.read(&mut data[0..4]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(4) => ReadState::process_len(data, conn),
                    Ok(idx) => {
                        ReadState::ReadingLen {
                            data,
                            idx: idx as u8,
                        }.next_state(conn)
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::Idle)
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
            ReadState::ReadingLen { mut data, mut idx } => {
                match conn.read(&mut data[(idx as usize)..4]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(amnt) if idx + amnt as u8 == 4 => ReadState::process_len(data, conn),
                    Ok(amnt) => {
                        idx += amnt as u8;
                        ReadState::ReadingLen { data, idx }.next_state(conn)
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::ReadingLen { data, idx })
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
            ReadState::ReadingId { mut data, len } => {
                match conn.read(&mut data[4..5]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(1) => ReadState::process_id(data, len, conn),
                    Ok(_) => unreachable!(),
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::ReadingId { data, len })
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
            ReadState::ReadingMsg {
                mut data,
                mut idx,
                len,
            } => {
                match conn.read(&mut data[(idx as usize)..(len + 4) as usize]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(amnt) if (amnt as u8 + idx - 4) as u32 == len => {
                        ReadState::process_message(data, len)
                    }
                    Ok(amnt) => {
                        idx += amnt as u8;
                        ReadState::ReadingMsg { data, idx, len }.next_state(conn)
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::ReadingMsg { data, idx, len })
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
            ReadState::ReadingPiece {
                mut prefix,
                mut data,
                len,
                mut idx,
            } => {
                if idx < 13 {
                    match conn.read(&mut prefix[idx as usize..13]) {
                        Ok(0) => ReadRes::EOF,
                        Ok(amnt) => {
                            idx += amnt;
                            ReadState::ReadingPiece {
                                prefix,
                                data,
                                len,
                                idx,
                            }.next_state(conn)
                        }
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                            ReadRes::Incomplete(ReadState::ReadingPiece {
                                prefix,
                                data,
                                len,
                                idx,
                            })
                        }
                        Err(e) => ReadRes::Err(e),
                    }
                } else {
                    match conn.read(&mut data[(idx - 13)..]) {
                        Ok(0) => ReadRes::EOF,
                        Ok(amnt) if idx + amnt - 13 == len as usize => {
                            let idx = (&prefix[5..9]).read_u32::<BigEndian>().unwrap();
                            let beg = (&prefix[9..13]).read_u32::<BigEndian>().unwrap();
                            ReadRes::Message(Message::Piece {
                                index: idx,
                                begin: beg,
                                length: len,
                                data,
                            })
                        }
                        Ok(amnt) => {
                            idx += amnt;
                            ReadState::ReadingPiece {
                                prefix,
                                data,
                                len,
                                idx,
                            }.next_state(conn)
                        }
                        Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                            ReadRes::Incomplete(ReadState::ReadingPiece {
                                prefix,
                                data,
                                len,
                                idx,
                            })
                        }
                        Err(e) => ReadRes::Err(e),
                    }
                }
            }
            ReadState::ReadingBitfield { mut data, mut idx } => {
                let len = data.len();
                match conn.read(&mut data[idx as usize..]) {
                    Ok(0) => ReadRes::EOF,
                    Ok(amnt) if idx + amnt == len => {
                        ReadRes::Message(Message::Bitfield(
                            Bitfield::from(data.into_boxed_slice(), len as u64 * 8),
                        ))
                    }
                    Ok(amnt) => {
                        idx += amnt;
                        ReadState::ReadingBitfield { data, idx }.next_state(conn)
                    }
                    Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                        ReadRes::Incomplete(ReadState::ReadingBitfield { data, idx })
                    }
                    Err(e) => ReadRes::Err(e),
                }
            }
        }
    }

    fn process_len<R: Read>(buf: [u8; 17], conn: &mut R) -> ReadRes {
        let len = (&buf[0..4]).read_u32::<BigEndian>().unwrap();
        if len == 0 {
            ReadRes::Message(Message::KeepAlive)
        } else {
            ReadState::ReadingId {
                data: buf,
                len: len,
            }.next_state(conn)
        }
    }

    fn process_id<R: Read>(buf: [u8; 17], len: u32, conn: &mut R) -> ReadRes {
        let id = buf[4];
        match id {
            0 => ReadRes::Message(Message::Choke),
            1 => ReadRes::Message(Message::Unchoke),
            2 => ReadRes::Message(Message::Interested),
            3 => ReadRes::Message(Message::Uninterested),
            5 => {
                ReadState::ReadingBitfield {
                    data: vec![0; len as usize - 1],
                    idx: 0,
                }.next_state(conn)
            }
            7 => {
                if len > 16_393 {
                    return ReadRes::Err(io_err_val(
                        "Only piece sizes of 16_384 or less are accepted",
                    ));
                }
                ReadState::ReadingPiece {
                    prefix: buf,
                    data: Box::new([0u8; 16_384]),
                    len: len - 9,
                    idx: 5,
                }.next_state(conn)
            }
            4 => {
                if len != 5 {
                    return ReadRes::Err(io_err_val("Invalid Have message length"));
                }
                ReadState::ReadingMsg {
                    data: buf,
                    idx: 5,
                    len: len,
                }.next_state(conn)
            }
            0x09 => {
                if len != 3 {
                    return ReadRes::Err(io_err_val("Invalid Port message length"));
                }
                ReadState::ReadingMsg {
                    data: buf,
                    idx: 5,
                    len: len,
                }.next_state(conn)
            }
            6 | 8 => {
                if len != 13 {
                    return ReadRes::Err(io_err_val("Invalid Request/Cancel message length"));
                }
                ReadState::ReadingMsg {
                    data: buf,
                    idx: 5,
                    len: len,
                }.next_state(conn)
            }
            _ => ReadRes::Err(io_err_val("Invalid ID provided!")),
        }
    }

    fn process_message(buf: [u8; 17], len: u32) -> ReadRes {
        match buf[4] {
            4 => {
                if len != 5 {
                    return ReadRes::Err(io_err_val("Have message must be of len 5"));
                }
                ReadRes::Message(Message::Have((&buf[5..9]).read_u32::<BigEndian>().unwrap()))
            }
            6 => {
                if len != 13 {
                    return ReadRes::Err(io_err_val("Request message must be of len 13"));
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>().unwrap();
                let beg = (&buf[9..13]).read_u32::<BigEndian>().unwrap();
                let len = (&buf[13..17]).read_u32::<BigEndian>().unwrap();
                ReadRes::Message(Message::Request {
                    index: idx,
                    begin: beg,
                    length: len,
                })
            }
            8 => {
                if len != 13 {
                    return ReadRes::Err(io_err_val("Cancel message must be of len 13"));
                }
                let idx = (&buf[5..9]).read_u32::<BigEndian>().unwrap();
                let beg = (&buf[9..13]).read_u32::<BigEndian>().unwrap();
                let len = (&buf[13..17]).read_u32::<BigEndian>().unwrap();
                ReadRes::Message(Message::Cancel {
                    index: idx,
                    begin: beg,
                    length: len,
                })
            }
            9 => {
                if len != 3 {
                    return ReadRes::Err(io_err_val("Port message must be of len 3"));
                }
                ReadRes::Message(Message::Port((&buf[5..7]).read_u16::<BigEndian>().unwrap()))
            }
            _ => ReadRes::Err(io_err_val("Invalid message ID")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Reader, ReadState};
    use torrent::peer::Message;
    use std::io::{self, Read};

    /// Cursor to emulate a mio socket using readv.
    struct Cursor<'a> {
        data: &'a [u8],
        idx: usize,
    }

    impl<'a> Cursor<'a> {
        fn new(data: &'a [u8]) -> Cursor {
            Cursor { data, idx: 0 }
        }
    }

    impl<'a> Read for Cursor<'a> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.idx >= self.data.len() {
                return Err(io::Error::new(io::ErrorKind::WouldBlock, ""));
            }
            let start = self.idx;
            for i in 0..buf.len() {
                if self.idx >= self.data.len() {
                    break;
                }
                buf[i] = self.data[self.idx];
                self.idx += 1;
            }
            Ok(self.idx - start)
        }
    }


    fn test_message(data: Vec<u8>, msg: Message) {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let mut data = Cursor::new(&data);
        assert_eq!(msg, r.readable(&mut data).unwrap().unwrap())
    }

    #[test]
    fn test_read_keepalive() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 0];
        test_message(data, Message::KeepAlive);
    }

    #[test]
    fn test_read_choke() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 1, 0];
        test_message(data, Message::Choke);
    }

    #[test]
    fn test_read_unchoke() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 1, 1];
        test_message(data, Message::Unchoke);
    }

    #[test]
    fn test_read_interested() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 1, 2];
        test_message(data, Message::Interested);
    }

    #[test]
    fn test_read_uninterested() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 1, 3];
        test_message(data, Message::Uninterested);
    }

    #[test]
    fn test_read_have() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 5, 4, 0, 0, 0, 1];
        test_message(data, Message::Have(1));
    }

    #[test]
    fn test_read_bitfield() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let v = vec![0u8, 0, 0, 5, 5, 0xff, 0xff, 0xff, 0xff];
        let mut data = Cursor::new(&v);
        // Test one shot
        match r.readable(&mut data).unwrap().unwrap() {
            Message::Bitfield(ref pf) => {
                for i in 0..32 {
                    assert!(pf.has_bit(i as u64));
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
        let v = vec![0u8, 0, 0, 13, 6, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1];
        let mut data = Cursor::new(&v);
        // Test one shot
        match r.readable(&mut data).unwrap().unwrap() {
            Message::Request {
                index,
                begin,
                length,
            } => {
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
        let mut v = vec![0u8, 0, 0x40, 0x09, 7, 0, 0, 0, 1, 0, 0, 0, 1];
        v.extend(vec![1u8; 16_384]);
        v.extend(vec![0u8, 0, 0x40, 0x09, 7, 0, 0, 0, 1, 0, 0, 0, 1]);
        v.extend(vec![1u8; 16_384]);

        let mut p1 = Cursor::new(&v[0..10]);
        let mut p2 = Cursor::new(&v[10..100]);
        let mut p3 = Cursor::new(&v[100..]);
        // Test partial read
        assert_eq!(r.readable(&mut p1).unwrap(), None);
        assert_eq!(r.readable(&mut p2).unwrap(), None);
        match r.readable(&mut p3).unwrap().unwrap() {
            Message::Piece {
                index,
                begin,
                length,
                ref data,
            } => {
                assert_eq!(index, 1);
                assert_eq!(begin, 1);
                assert_eq!(length, 16_384);
                for i in 0..16_384 {
                    assert_eq!(1, data[i]);
                }
            }
            _ => {
                unreachable!();
            }
        }
        match r.readable(&mut p3).unwrap().unwrap() {
            Message::Piece {
                index,
                begin,
                length,
                ref data,
            } => {
                assert_eq!(index, 1);
                assert_eq!(begin, 1);
                assert_eq!(length, 16_384);
                for i in 0..16_384 {
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
        let v = vec![0u8, 0, 0, 13, 8, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1];
        let mut data = Cursor::new(&v);
        // Test one shot
        match r.readable(&mut data).unwrap().unwrap() {
            Message::Cancel {
                index,
                begin,
                length,
            } => {
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
    fn test_read_port() {
        let mut r = Reader::new();
        r.state = ReadState::Idle;
        let data = vec![0u8, 0, 0, 3, 9, 0x1A, 0xE1];
        test_message(data, Message::Port(6881));
    }

    #[test]
    fn test_read_handshake() {
        use PEER_ID;
        let mut r = Reader::new();
        let m = Message::Handshake {
            rsv: [0; 8],
            hash: [0; 20],
            id: *PEER_ID,
        };
        let mut data = vec![0; 68];
        m.encode(&mut data[..]).unwrap();
        let mut c = Cursor::new(&data);
        assert_eq!(r.readable(&mut c).unwrap().unwrap(), m);
    }
}
