use std::io;
use std::result;
use tracker::errors::{Result, ErrorKind};

pub(super) struct Reader {
    data: Vec<u8>,
    idx: usize,
    state: ReadState,
}

enum ReadState {
    ParsingHeaders,
    ParsingResponse,
}

enum ReadRes {
    Done,
    Again,
    Empty,
}

impl ReadState {
    fn ready(&self) -> bool {
        match *self {
            ReadState::ParsingHeaders => false,
            ReadState::ParsingResponse => true,
        }
    }
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            data: Vec::with_capacity(75),
            idx: 0,
            state: ReadState::ParsingHeaders,
        }
    }

    pub fn readable<R: io::Read>(&mut self, conn: &mut R) -> Result<bool> {
        while let ReadRes::Again = self.read(conn)? { }
        Ok(self.state.ready())
    }

    fn read<R: io::Read>(&mut self, conn: &mut R) -> Result<ReadRes> {
        match conn.read(&mut self.data[self.idx..]) {
            Ok(0) if self.state.ready() => Ok(ReadRes::Done),
            Ok(0) => Err(ErrorKind::EOF.into()),
            Ok(v) => {
                if self.state.ready() {
                    self.idx += v;
                    if self.idx == self.data.len() {
                        self.data.resize(self.idx + 30, 0);
                    }
                } else {
                    for i in 0..v {
                        if self.data[i..i+4] == b"\r\n\r\n"[..] {
                            self.data = self.data.split_off(i+4);
                            self.state = ReadState::ParsingResponse;
                            break;
                        }
                    }
                }
                Ok(ReadRes::Again)
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock {
                    Ok(ReadRes::Empty)
                } else {
                    return Err(ErrorKind::IO.into());
                }
            }
        }
    }

    pub fn consume(self) -> Vec<u8> {
        self.data
    }
}
