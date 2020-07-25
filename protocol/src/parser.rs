use crate::Message;
use crate::Buffer;
use crate::Bitfield;

use byteorder::{BigEndian, ByteOrder};

use std::{io, mem};
use std::marker::PhantomData;

struct Processor <BF: Bitfield, Buf: Buffer> {
    buf_idx: usize,
    bf_type: PhantomData<BF>,
    buf_type: PhantomData<Buf>,
    state: State<BF, Buf>,
}

enum State<BF: Bitfield, Buf: Buffer> {
    Empty,
    Prefix { data: [u8; 17] },
    Handshake { data: [u8; 68] },
    Piece { data: Buf },
    Bitfield { data: Vec<u8> },
    Extension { id: u8, data: Vec<u8> },
    Phantom { phantom: PhantomData<BF> },
}

enum ParseResult<BF: Bitfield, Buf: Buffer> {
    Success { msg: Message<BF, Buf>, consumed: usize, state: State<BF, Buf> },
    Incomplete { state: State<BF, Buf> },
    Stalled { state: State<BF, Buf> },
    Error,
}

struct Reader {
}

impl <BF: Bitfield, Buf: Buffer> Processor<BF, Buf> {
    fn new() -> Processor<BF, Buf> {
        Processor {
            buf_idx: 0,
            bf_type: PhantomData,
            buf_type: PhantomData,
            state: State::Handshake { data: [0u8; 68] }
        }
    }

    fn buf_mut(&mut self) -> &mut [u8] {
        let buf = match &mut self.state {
            State::Prefix { data } => &mut data[..],
            State::Handshake { data } => &mut data[..],
            State::Piece { ref mut data } => data,
            State::Bitfield { ref mut data } => data,
            State::Extension { ref mut data, .. } => data,
            State::Empty | State::Phantom { .. } => unreachable!(),
        };
        &mut buf[self.buf_idx..]
    }

    fn push_data(&mut self, len: usize)  {
        self.buf_idx += len;
        let prev_state = mem::replace(&mut self.state, State::Empty);
        let result = prev_state.parse(self.buf_idx);
        // self.state = state;
        // res
    }
}

impl <BF: Bitfield, Buf: Buffer> State<BF, Buf> {
    fn new_prefix() -> State<BF, Buf> {
        State::Prefix { data: [0u8; 17] }
    }
    fn new_handshake() -> State<BF, Buf> {
        State::Handshake { data: [0u8; 68] }
    }
    fn new_piece() -> Option<State<BF, Buf>> {
        Buf::get().map(|buf| State::Piece { data: buf })
    }
    // fn new_bitfield() -> State<BF, Buf> {

    fn parse(mut self, len: usize) -> ParseResult<BF, Buf> {
        match &mut self {
            State::Prefix { data } => {
                if len <= 4 {
                    return ParseResult::Incomplete { state: self }
                }
             let msg_len = BigEndian::read_u32(&data[0..4]);
             if msg_len == 0 {
                 return ParseResult::Success { msg: Message::KeepAlive, consumed: 4, state: State::new_prefix() };
             }
             let msg_id = data[4];
             match msg_id {
                 0 => ParseResult::Success { msg: Message::Choke, consumed: 5, state: State::new_prefix() },
                 /*
                                let msg = if id == 0 {
                                    Message::Choke
                                } else if id == 1 {
                                    Message::Unchoke
                                } else if id == 2 {
                                    Message::Interested
                                } else {
                                    Message::Uninterested
                                };
                                */
             }
             return ParseResult::Incomplete { state: self }
            }
            _ => unreachable!(),
        }
    }
}
