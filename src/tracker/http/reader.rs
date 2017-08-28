use std::io;
use std::mem;
use tracker::errors::{Result, ErrorKind};

pub struct Reader {
    data: Vec<u8>,
    idx: usize,
    state: ReadState,
}

enum ReadState {
    HeaderR1,
    HeaderN1,
    HeaderR2,
    HeaderN2,
    Response,
}

enum ReadRes {
    Done,
    Again,
    Empty,
}

impl ReadState {
    fn handle(&mut self, byte: u8) -> bool {
        let s = mem::replace(self, ReadState::HeaderR1);
        mem::replace(self, s.next(byte));
        self.ready()
    }

    fn next(self, byte: u8) -> ReadState {
        // We organize like this to make the state machine more clear
        #[cfg_attr(feature = "cargo-clippy", allow(match_same_arms))]
        match (self, byte) {
            (ReadState::HeaderR1, b'\r') => ReadState::HeaderN1,

            (ReadState::HeaderN1, b'\r') => ReadState::HeaderN1,
            (ReadState::HeaderN1, b'\n') => ReadState::HeaderR2,

            (ReadState::HeaderR2, b'\r') => ReadState::HeaderN2,

            (ReadState::HeaderN2, b'\r') => ReadState::HeaderN1,
            (ReadState::HeaderN2, b'\n') => ReadState::Response,

            _ => ReadState::HeaderR1,
        }
    }

    fn ready(&self) -> bool {
        match *self {
            ReadState::Response => true,
            _ => false,
        }
    }
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            data: vec![0; 75],
            idx: 0,
            state: ReadState::HeaderN1,
        }
    }

    pub fn readable<R: io::Read>(&mut self, conn: &mut R) -> Result<bool> {
        while let ReadRes::Again = self.read(conn)? {}
        Ok(self.state.ready())
    }

    fn read<R: io::Read>(&mut self, conn: &mut R) -> Result<ReadRes> {
        match conn.read(&mut self.data[self.idx..]) {
            Ok(0) if self.state.ready() => Ok(ReadRes::Done),
            Ok(0) => Err(ErrorKind::EOF.into()),
            Ok(v) => {
                if self.state.ready() {
                    self.idx += v;
                } else {
                    for i in 0..v {
                        if self.state.handle(self.data[i]) {
                            self.data = self.data.split_off(i + 1);
                            self.idx = v - (i + 1);
                            break;
                        }
                    }
                }
                if self.idx == self.data.len() {
                    self.data.resize(self.idx + 30, 0);
                }
                Ok(ReadRes::Again)
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    Ok(ReadRes::Empty)
                } else {
                    Err(ErrorKind::IO.into())
                }
            }
        }
    }

    pub fn consume(mut self) -> Vec<u8> {
        self.data.truncate(self.idx);
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::Reader;
    use std::io::Cursor;

    #[test]
    fn test_empty_resp() {
        let mut r = Reader::new();
        let data = "SomeHeader: Foo\r\nConnection: Close\r\n\r\n";
        let mut c = Cursor::new(data);
        assert_eq!(r.readable(&mut c).unwrap(), true);
        assert_eq!(r.consume(), Vec::<u8>::new());
    }

    #[test]
    fn test_premature_resp() {
        let mut r = Reader::new();
        let data = "SomeHeader: Foo\r\nConnection: C";
        let mut c = Cursor::new(data);
        assert_eq!(r.readable(&mut c).is_err(), true);
    }

    #[test]
    fn test_valid_resp() {
        let mut r = Reader::new();
        let data = "SomeHeader: Foo\r\nConnection: Close\r\n\r\nhello world spam";
        let mut c = Cursor::new(data);
        assert_eq!(r.readable(&mut c).unwrap(), true);
        assert_eq!(r.consume(), b"hello world spam");
    }
}
