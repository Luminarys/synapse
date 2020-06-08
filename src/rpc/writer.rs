use super::proto::ws::Message;
use crate::util::{awrite, IOR};
use std::collections::VecDeque;
use std::io;

// TODO: Consider how to handle larger streamed messages(maybe)
// may be better to just offer an http interface for chunked DL anyways
pub struct Writer {
    queue: VecDeque<Message>,
    state: State,
}

enum State {
    Idle,
    Writing { pos: usize, buf: Vec<u8> },
}

enum WR {
    Incomplete,
    Complete,
    Blocked,
}

impl Writer {
    pub fn new() -> Writer {
        Writer {
            queue: VecDeque::new(),
            state: State::Idle,
        }
    }

    pub fn write<W: io::Write>(&mut self, w: &mut W) -> io::Result<()> {
        loop {
            match self.do_write(w)? {
                WR::Complete => {
                    self.next_msg();
                    if self.state.idle() {
                        return Ok(());
                    }
                }
                WR::Incomplete => {}
                WR::Blocked => return Ok(()),
            }
        }
    }

    fn do_write<W: io::Write>(&mut self, w: &mut W) -> io::Result<WR> {
        match self.state {
            State::Idle => Ok(WR::Complete),
            State::Writing {
                ref mut pos,
                ref buf,
            } => match awrite(&buf[*pos..], w) {
                IOR::Complete => Ok(WR::Complete),
                IOR::Incomplete(a) => {
                    *pos += a;
                    Ok(WR::Incomplete)
                }
                IOR::Blocked => Ok(WR::Blocked),
                IOR::EOF => Err(io::ErrorKind::UnexpectedEof.into()),
                IOR::Err(e) => Err(e),
            },
        }
    }

    fn next_msg(&mut self) {
        match self.queue.pop_front() {
            Some(m) => {
                self.state = State::Writing {
                    pos: 0,
                    buf: m.serialize(),
                }
            }
            None => self.state = State::Idle,
        }
    }

    pub fn enqueue(&mut self, msg: Message) {
        if self.state.idle() {
            self.state = State::Writing {
                pos: 0,
                buf: msg.serialize(),
            }
        } else {
            self.queue.push_back(msg);
        }
    }
}

impl State {
    fn idle(&self) -> bool {
        match *self {
            State::Idle => true,
            _ => false,
        }
    }
}
