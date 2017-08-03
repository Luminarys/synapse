use std::{io, mem};
use byteorder::{BigEndian, ReadBytesExt};
use super::proto::Message;

pub struct Reader {
    msg: Message,
    pos: usize,
    state: State,
}

#[derive(Copy, Clone)]
enum State {
    Header,
    PayloadLen2,
    PayloadLen8,
    MaskingKey,
    Payload(usize),
}

enum RR {
    Complete,
    Incomplete(usize),
    Blocked,
    EOF,
    Err(io::Error),
}

impl Reader {
    pub fn read<R: io::Read>(&mut self, r: &mut R) -> io::Result<Option<Message>> {
        loop {
            let start = 0;
            let end = self.state.size();

            match (do_read(&mut self.msg.data[self.pos..end], r), self.state) {
                (RR::Blocked, _) => {
                    return Ok(None);
                }

                (RR::EOF, _) => {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                (RR::Err(e), _) => {
                    return Err(e);
                }

                (RR::Incomplete(a), _) => {
                    self.pos += a;
                }

                (RR::Complete, State::Header) => {
                    self.msg.header = self.msg.data[start];
                    if self.msg.data[start+1] & 0x80 != 0 {
                        self.msg.mask = Some(0);
                    }

                    match self.msg.data[start+1] & 0x7f {
                        126 => {
                            self.state = State::PayloadLen2;
                        }
                        127 => {
                            self.state = State::PayloadLen8;
                        }
                        l => {
                            self.msg.len = l as u64;
                            if self.msg.masked() {
                                self.state = State::MaskingKey;
                            } else {
                                self.state = State::Payload(l as usize);
                                self.msg.allocate();
                            }
                        }
                    }

                    self.pos = 0;
                }

                (RR::Complete, State::PayloadLen2) | (RR::Complete, State::PayloadLen8) => {
                    let mut buf = &self.msg.data[start..end];
                    match self.state {
                        State::PayloadLen2 => self.msg.len = buf.read_u16::<BigEndian>().unwrap() as u64,
                        State::PayloadLen8 => self.msg.len = buf.read_u64::<BigEndian>().unwrap(),
                        _ => unreachable!(),
                    }
                    self.msg.allocate();

                    if self.msg.masked() {
                        self.state = State::MaskingKey;
                    } else {
                        self.state = State::Payload(self.msg.len as usize);
                    }

                    self.pos = 0;
                }

                (RR::Complete, State::MaskingKey) => {
                    self.msg.mask = Some((&self.msg.data[start..end]).read_u32::<BigEndian>().unwrap());
                    self.state = State::Payload(self.msg.len as usize);

                    self.pos = 0;
                }

                (RR::Complete, State::Payload(l)) => {
                    self.state = State::Header;

                    self.pos = 0;

                    return Ok(Some(mem::replace(&mut self.msg, Message::new())));
                }
            }
        }
    }
}

impl State {
    pub fn size(&self) -> usize {
        match *self {
            State::Header => 2,
            State::PayloadLen2 => 2,
            State::PayloadLen8 => 8,
            State::MaskingKey => 4,
            State::Payload(s) => s,
        }
    }
}

fn do_read<R: io::Read>(b: &mut [u8], r: &mut R) -> RR {
    match r.read(b) {
        Ok(0) => RR::EOF,
        Ok(a) if a == b.len() => RR::Complete,
        Ok(a)  => RR::Incomplete(a),
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => RR::Blocked,
        Err(e) => RR::Err(e),
    }
}
