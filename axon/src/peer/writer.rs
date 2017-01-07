use message::Message;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::Arc;

pub struct Writer {
    chunks_written: usize,
    writable: bool,
    write_queue: VecDeque<Message>,
    state: WriteState,
}

enum WriteState {
    Idle,
    WritingMsg { data: [u8; 13], len: u8, idx: u8 },
    WritingOther { data: Vec<u8>, idx: u16 },
    WritingPiece { prefix: [u8; 13], data: Arc<[u8; 16384]>, idx: u16 }
}

impl Writer {
    pub fn new() -> Writer {
        Writer {
            writable: false,
            write_queue: VecDeque::new(),
            state: WriteState::Idle,
            chunks_written: 0,
        }
    }

    // TODO: Make generic over Write trait for testing purposes, then use futures::sync::BiLock to create Read/WRite
    // pair for struct ownership
    pub fn writable<W: Write>(&mut self, conn: &mut W) -> io::Result<()> {
        self.writable = true;
        self.write(conn)
    }

    pub fn write_message<W: Write>(&mut self, msg: Message, conn: &mut W) -> io::Result<()> {
        if let WriteState::Idle = self.state {
            self.setup_write(msg);
        } else {
            self.write_queue.push_back(msg);
        }
        if self.writable {
            self.write(conn)
        } else {
            Ok(())
        }
    }

    pub fn chunks_written(&self) -> usize {
        self.chunks_written
    }

    fn setup_write(&mut self, msg: Message) {
        self.state = if !msg.is_special() {
            let mut buf = [0; 13];
            let len = msg.len();
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            match msg {
                Message::SharedPiece(_, _, data) => {
                    WriteState::WritingPiece { prefix: buf, data: data, idx: 0 }
                }
                _ => {
                    WriteState::WritingMsg { data: buf, len: len as u8, idx: 0 }
                }
            }
        } else {
            let mut buf = Vec::with_capacity(msg.len());
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            WriteState::WritingOther { data: buf, idx: 0 }
        };
    }

    fn write<W: Write>(&mut self, conn: &mut W) -> io::Result<()> {
        while self.write_(conn)? {
            if let Some(msg) = self.write_queue.pop_back() {
                self.setup_write(msg);
            } else {
                break;
            }
        }
        Ok(())
    }

    fn write_<W: Write>(&mut self, conn: &mut W) -> io::Result<bool> {
        match self.state {
            WriteState::Idle => Ok(false),
            WriteState::WritingMsg { ref data, ref len, ref mut idx } => {
                let amnt = conn.write(&data[(*idx as usize)..(*len as usize)])?;
                *idx += amnt as u8;
                if idx == len {
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingPiece { ref prefix, ref data, ref mut idx } => {
                if *idx < prefix.len() as u16 {
                    let amnt = conn.write(&prefix[(*idx as usize)..])?;
                    *idx += amnt as u16;
                    if *idx != prefix.len() as u16 {
                        self.writable = false;
                        return Ok(false);
                    }
                }

                let amnt = conn.write(&prefix[(*idx as usize - prefix.len())..])?;
                // piece should never exceed u16 size
                *idx += amnt as u16;
                if *idx == (prefix.len() + data.len()) as u16 {
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingOther { ref data, ref mut idx } => {
                let amnt = conn.write(&data[(*idx as usize)..])?;
                *idx += amnt as u16;
                if *idx == data.len() as u16 {
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
        }
    }
}
