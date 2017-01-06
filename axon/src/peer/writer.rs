use message::Message;
use std::collections::VecDeque;
use std::io::{self, Write};
use mio::tcp::TcpStream;

pub struct Writer {
    chunks_written: usize,
    writable: bool,
    write_queue: VecDeque<Message>,
    state: WriteState,
}

enum WriteState {
    Idle,
    WritingMsg { data: [u8; 13], len: u8, idx: u8 },
    WritingOther { data: Vec<u8>, idx: u8 },
    WritingPiece { prefix: [u8; 13], data: Arc<[u8; 16384]>, idx: u16 }
}

impl Writer {
    pub fn new() -> Writer {
        Writer {
            writable: false,
            write_queue: VecDeque::new(),
            state: WriteState::Idle,
        }
    }

    // TODO: Make generic over Write trait for testing purposes, then use futures::sync::BiLock to create Read/WRite
    // pair for struct ownership
    pub fn writable(&mut self, conn: &mut TcpStream) -> io::Result<()> {
        self.writable = true;
        self.write(conn)
    }

    pub fn write_message(&mut self, msg: Message, conn: &mut TcpStream) -> io::Result<()> {
        if let WriteState::Idle == self.state {
            self.setup_write(msg);
        } else {
            self.write_queue.push_back(msg);
        }
        if self.writable {
            self.write()
        } else {
            Ok(())
        }
    }

    pub fn chunks_written(&self) -> usize {
        self.chunks_written
    }

    fn setup_write(&mut self, msg: Message) {
        self.state = if !msg.is_special() {
            let buf = [u8; 13];
            let len = msg.len();
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            match msg {
                Message::SharedPiece(_, _, data) => {
                    WriteState::WritingPiece { prefix: buf, data: data, idx: 0 }
                }
                _ => {
                    WriteState::WritingMsg { data: buf, len: len, idx: 0 }
                }
            }
        } else {
            let buf = Vec::with_capacity(msg.len());
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            WriteState::WritingOther { data: buf, idx: 0 }
        };
    }

    fn write(&mut self, conn: &mut TcpStream) -> io::Result<()> {
        while self.write_(conn)? {
            if let Some(msg) = self.write_queue.pop_back() {
                self.setup_write(msg)?;
            } else {
                break;
            }
        }
        Ok(())
    }

    fn write_(&mut self, conn: &mut TcpStream) -> io::Result<bool> {
        match self.state {
            WriteState::Idle => Ok(false),
            WriteState::WritingMsg { ref data, ref len, ref mut idx } => {
                let amnt = conn.write(&data[idx..len])?;
                if idx + amnt == len {
                    Ok(true)
                } else {
                    idx += amnt;
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingPiece { ref prefix, ref data, ref mut idx } => {
                if idx < prefix.len() {
                    let amnt = conn.write(&prefix[idx..])?;
                    idx += amnt;
                    if idx != prefix.len() {
                        self.writable = false;
                        return Ok(false);
                    }
                }

                let amnt = conn.write(&data[(idx - prefix.len())..]);
                idx += amnt;
                if idx == prefix.len() + data.len() {
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingOther { ref data, ref mut idx } => {
                let amnt = conn.write(&data[idx..])?;
                if idx + amnt == len {
                    Ok(true)
                } else {
                    idx += amnt;
                    self.writable = false;
                    Ok(false)
                }
            }
        }
    }
}
