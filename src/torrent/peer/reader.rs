use std::io::{self, Read};
use std::mem;
use torrent::peer::Message;
use torrent::Bitfield;
use byteorder::{BigEndian, ReadBytesExt};
use util::{aread, IOR, io_err};

pub struct Reader {
    blocks_read: usize,

    state: State,
    prefix: [u8; 17],
    idx: usize,
}

enum State {
    Len,
    ID,
    Have,
    Request,
    Cancel,
    Port,
    Handshake { data: [u8; 68] },
    PiecePrefix,
    Piece {
        data: Option<Box<[u8; 16_384]>>,
        len: u32,
    },
    Bitfield { data: Vec<u8> },
    ExtensionID,
    Extension { id: u8, payload: Vec<u8> },
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            blocks_read: 0,
            prefix: [0u8; 17],
            idx: 0,
            state: State::Handshake { data: [0u8; 68] },
        }
    }

    pub fn readable<R: Read>(&mut self, conn: &mut R) -> io::Result<Option<Message>> {
        let res = self.readable_(conn);
        if res.as_ref().ok().map(|o| o.is_some()).unwrap_or(false) {
            self.state = State::Len;
            self.idx = 0;
        }
        res
    }

    fn readable_<R: Read>(&mut self, conn: &mut R) -> io::Result<Option<Message>> {
        loop {
            let len = self.state.len();
            match self.state {
                State::Handshake { ref mut data } => {
                    match aread(&mut data[self.idx..len], conn) {
                        IOR::Complete => {
                            if &data[1..20] != b"BitTorrent protocol" {
                                return io_err("Handshake was not for 'BitTorrent protocol'");
                            }
                            let mut rsv = [0; 8];
                            rsv.clone_from_slice(&data[20..28]);
                            let mut hash = [0; 20];
                            hash.clone_from_slice(&data[28..48]);
                            let mut id = [0; 20];
                            id.clone_from_slice(&data[48..68]);

                            return Ok(Some(Message::Handshake { rsv, hash, id }));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Len => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let mlen = (&self.prefix[0..4]).read_u32::<BigEndian>().unwrap();
                            if mlen == 0 {
                                return Ok(Some(Message::KeepAlive));
                            } else {
                                self.idx = 4;
                                self.state = State::ID;
                            }
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::ID => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            self.idx = 5;
                            match self.prefix[4] {
                                0...3 => {
                                    let id = self.prefix[4];
                                    let msg = if id == 0 {
                                        Message::Choke
                                    } else if id == 1 {
                                        Message::Unchoke
                                    } else if id == 2 {
                                        Message::Interested
                                    } else {
                                        Message::Uninterested
                                    };
                                    return Ok(Some(msg));
                                }
                                4 => self.state = State::Have,
                                5 => {
                                    let mlen =
                                        (&self.prefix[0..4]).read_u32::<BigEndian>().unwrap();
                                    self.idx = 0;
                                    self.state =
                                        State::Bitfield { data: vec![0u8; mlen as usize - 1] };
                                }
                                6 => self.state = State::Request,
                                7 => self.state = State::PiecePrefix,
                                8 => self.state = State::Cancel,
                                9 => self.state = State::Port,
                                20 => self.state = State::ExtensionID,
                                _ => return io_err("Invalid ID used!"),
                            }
                        }
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                        IOR::Incomplete(_) => unreachable!(),
                    }
                }
                State::Have => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let have = (&self.prefix[5..9]).read_u32::<BigEndian>().unwrap();
                            return Ok(Some(Message::Have(have)));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Bitfield { ref mut data } => {
                    match aread(&mut data[self.idx..len], conn) {
                        IOR::Complete => {
                            let d = mem::replace(data, vec![]).into_boxed_slice();
                            let bf = Bitfield::from(d, len as u64 * 8);
                            return Ok(Some(Message::Bitfield(bf)));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Request => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let index = (&self.prefix[5..9]).read_u32::<BigEndian>().unwrap();
                            let begin = (&self.prefix[9..13]).read_u32::<BigEndian>().unwrap();
                            let length = (&self.prefix[13..17]).read_u32::<BigEndian>().unwrap();
                            return Ok(Some(Message::Request {
                                index,
                                begin,
                                length,
                            }));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::PiecePrefix => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let plen = (&self.prefix[0..4]).read_u32::<BigEndian>().unwrap() - 9;
                            self.idx = 0;
                            self.state = State::Piece {
                                data: Some(Box::new(unsafe { mem::uninitialized() })),
                                len: plen,
                            };
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Piece {
                    ref mut data,
                    len: length,
                } => {
                    match aread(&mut data.as_mut().unwrap()[self.idx..len], conn) {
                        IOR::Complete => {
                            self.blocks_read += 1;
                            let index = (&self.prefix[5..9]).read_u32::<BigEndian>().unwrap();
                            let begin = (&self.prefix[9..13]).read_u32::<BigEndian>().unwrap();
                            return Ok(Some(Message::Piece {
                                index,
                                begin,
                                length,
                                data: mem::replace(data, None).unwrap(),
                            }));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Cancel => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let index = (&self.prefix[5..9]).read_u32::<BigEndian>().unwrap();
                            let begin = (&self.prefix[9..13]).read_u32::<BigEndian>().unwrap();
                            let length = (&self.prefix[13..17]).read_u32::<BigEndian>().unwrap();
                            return Ok(Some(Message::Cancel {
                                index,
                                begin,
                                length,
                            }));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::Port => {
                    match aread(&mut self.prefix[self.idx..len], conn) {
                        IOR::Complete => {
                            let port = (&self.prefix[5..7]).read_u16::<BigEndian>().unwrap();
                            return Ok(Some(Message::Port(port)));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
                State::ExtensionID => {
                    match aread(&mut self.prefix[5..6], conn) {
                        IOR::Complete => {
                            let id = self.prefix[5];
                            self.idx = 0;
                            let plen = (&self.prefix[0..4]).read_u32::<BigEndian>().unwrap() - 2;
                            let mut payload = Vec::with_capacity(plen as usize);
                            unsafe {
                                payload.set_len(plen as usize);
                            }
                            self.state = State::Extension { id, payload };
                        }
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                        IOR::Incomplete(_) => unreachable!(),
                    }
                }
                State::Extension {
                    id,
                    ref mut payload,
                } => {
                    match aread(&mut payload[self.idx..len], conn) {
                        IOR::Complete => {
                            let p = mem::replace(payload, Vec::with_capacity(0));
                            return Ok(Some(Message::Extension { id, payload: p }));
                        }
                        IOR::Incomplete(a) => self.idx += a,
                        IOR::Blocked => return Ok(None),
                        IOR::EOF => return io_err("EOF"),
                        IOR::Err(e) => return Err(e),
                    }
                }
            }
        }
    }
}

impl State {
    fn len(&self) -> usize {
        match *self {
            State::Len => 4,
            State::ID => 5,
            State::Have => 9,
            State::Request | State::Cancel => 17,
            State::PiecePrefix => 13,
            State::Port => 7,
            State::Handshake { .. } => 68,
            State::Piece { len, .. } => len as usize,
            State::Bitfield { ref data, .. } => data.len(),
            State::ExtensionID => 6,
            State::Extension { ref payload, .. } => payload.len(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        r.state = State::Len;
        let mut data = Cursor::new(&data);
        assert_eq!(msg, r.readable(&mut data).unwrap().unwrap())
    }

    #[test]
    fn test_read_keepalive() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 0];
        test_message(data, Message::KeepAlive);
    }

    #[test]
    fn test_read_choke() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 1, 0];
        test_message(data, Message::Choke);
    }

    #[test]
    fn test_read_unchoke() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 1, 1];
        test_message(data, Message::Unchoke);
    }

    #[test]
    fn test_read_interested() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 1, 2];
        test_message(data, Message::Interested);
    }

    #[test]
    fn test_read_uninterested() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 1, 3];
        test_message(data, Message::Uninterested);
    }

    #[test]
    fn test_read_have() {
        let mut r = Reader::new();
        r.state = State::Len;
        let data = vec![0u8, 0, 0, 5, 4, 0, 0, 0, 1];
        test_message(data, Message::Have(1));
    }

    #[test]
    fn test_read_bitfield() {
        let mut r = Reader::new();
        r.state = State::Len;
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
        r.state = State::Len;
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
        r.state = State::Len;
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
        r.state = State::Len;
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
        r.state = State::Len;
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
