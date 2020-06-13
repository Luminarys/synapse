use super::proto::ws::Message;
use crate::util::{aread, IOR};
use byteorder::{BigEndian, ByteOrder};
use std::{io, mem};

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

impl Reader {
    pub fn new() -> Reader {
        Reader {
            msg: Message::new(),
            pos: 0,
            state: State::Header,
        }
    }

    pub fn read<R: io::Read>(&mut self, r: &mut R) -> io::Result<Option<Message>> {
        loop {
            let start = 0;
            let end = self.state.size();
            match (aread(&mut self.msg.data[self.pos..end], r), self.state) {
                (IOR::Blocked, _) => {
                    return Ok(None);
                }

                (IOR::EOF, _) => {
                    return Err(io::ErrorKind::UnexpectedEof.into());
                }

                (IOR::Err(e), _) => {
                    return Err(e);
                }

                (IOR::Incomplete(a), State::Payload(_)) => {
                    for i in self.pos..self.pos + a {
                        self.msg.data[i] ^= self.msg.mask.unwrap()[i % 4];
                    }
                    self.pos += a;
                }

                (IOR::Incomplete(a), _) => {
                    self.pos += a;
                }

                (IOR::Complete, State::Header) => {
                    self.msg.header = self.msg.data[start];
                    if self.msg.data[start + 1] & 0x80 != 0 {
                        self.msg.mask = Some([0; 4]);
                    }

                    match self.msg.data[start + 1] & 0x7f {
                        126 => {
                            self.state = State::PayloadLen2;
                        }
                        127 => {
                            self.state = State::PayloadLen8;
                        }
                        l => {
                            self.msg.len = u64::from(l);
                            if self.msg.masked() {
                                self.state = State::MaskingKey;
                            } else {
                                self.msg.allocate();
                                self.state = State::Payload(l as usize);
                            }
                        }
                    }

                    self.pos = 0;
                }

                (IOR::Complete, State::PayloadLen2) | (IOR::Complete, State::PayloadLen8) => {
                    {
                        let buf = &self.msg.data[start..end];
                        match self.state {
                            State::PayloadLen2 => {
                                self.msg.len = u64::from(BigEndian::read_u16(buf))
                            }
                            State::PayloadLen8 => self.msg.len = BigEndian::read_u64(buf),
                            _ => unreachable!(),
                        }
                    }
                    if self.msg.masked() {
                        self.state = State::MaskingKey;
                    } else {
                        self.msg.allocate();
                        self.state = State::Payload(self.msg.len as usize);
                    }

                    self.pos = 0;
                }

                (IOR::Complete, State::MaskingKey) => {
                    let mut mask = [0; 4];
                    mask.copy_from_slice(&self.msg.data[start..end]);
                    self.msg.mask = Some(mask);
                    self.state = State::Payload(self.msg.len as usize);
                    self.msg.allocate();

                    self.pos = 0;
                }

                (IOR::Complete, State::Payload(_)) => {
                    for i in self.pos..end {
                        self.msg.data[i] ^= self.msg.mask.unwrap()[i % 4];
                    }

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
