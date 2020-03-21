use std::collections::VecDeque;
use std::io::{self, ErrorKind, Write};

use buffers::Buffer;
use torrent::peer::Message;
use util::io_err;

pub struct Writer {
    // Needed so that the peer can filter out cancel'd messages.
    // The state of this isn't critical to any invariants of the Writer
    // so it shouldn't be an issue
    pub write_queue: VecDeque<Message>,
    blocks_written: usize,
    writable: bool,
    state: WriteState,
}

enum WriteState {
    Idle,
    WritingMsg {
        data: [u8; 17],
        len: u8,
        idx: u8,
    },
    WritingOther {
        data: Vec<u8>,
        idx: u16,
    },
    WritingPiece {
        prefix: [u8; 17],
        data: Buffer,
        idx: u16,
    },
}

impl Writer {
    pub fn new() -> Writer {
        Writer {
            writable: true,
            write_queue: VecDeque::new(),
            state: WriteState::Idle,
            blocks_written: 0,
        }
    }

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

    fn setup_write(&mut self, msg: Message) {
        self.state = if !msg.is_special() {
            let mut buf = [0; 17];
            let len = msg.len();
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            match msg {
                Message::Piece { data, .. } => WriteState::WritingPiece {
                    prefix: buf,
                    data,
                    idx: 0,
                },
                _ => WriteState::WritingMsg {
                    data: buf,
                    len: len as u8,
                    idx: 0,
                },
            }
        } else {
            // TODO: Acquire from buffer
            let mut buf = vec![0; msg.len()];
            // Should never go wrong
            msg.encode(&mut buf).unwrap();
            WriteState::WritingOther { data: buf, idx: 0 }
        };
    }

    fn write<W: Write>(&mut self, conn: &mut W) -> io::Result<()> {
        if let WriteState::Idle = self.state {
            return Ok(());
        }
        loop {
            match self.write_(conn) {
                Ok(true) => {
                    if let Some(msg) = self.write_queue.pop_back() {
                        self.setup_write(msg);
                    } else {
                        self.state = WriteState::Idle;
                        break;
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock
                        || e.kind() == ErrorKind::NotConnected
                        || e.kind() == ErrorKind::BrokenPipe
                    {
                        break;
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        Ok(())
    }

    fn write_<W: Write>(&mut self, conn: &mut W) -> io::Result<bool> {
        match self.state {
            WriteState::Idle => Ok(false),
            WriteState::WritingMsg {
                ref data,
                ref len,
                ref mut idx,
            } => {
                let amnt = conn.write(&data[(*idx as usize)..(*len as usize)])?;
                if amnt == 0 {
                    return io_err("EOF");
                }
                *idx += amnt as u8;
                if idx == len {
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingPiece {
                ref prefix,
                ref data,
                ref mut idx,
            } => {
                if *idx < 13 as u16 {
                    let amnt = conn.write(&prefix[(*idx as usize)..13])? as u16;
                    if amnt == 0 {
                        return io_err("EOF");
                    }
                    *idx += amnt;
                    if *idx != 13 as u16 {
                        self.writable = false;
                        return Ok(false);
                    }
                }

                let amnt = conn.write(&data[(*idx as usize - 13)..])?;
                if amnt == 0 {
                    return io_err("EOF");
                }
                // piece should never exceed u16 size
                *idx += amnt as u16;
                if *idx == (13 + data.len()) as u16 {
                    self.blocks_written += 1;
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingOther {
                ref data,
                ref mut idx,
            } => {
                let amnt = conn.write(&data[(*idx as usize)..])?;
                if amnt == 0 {
                    return io_err("EOF");
                }
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

#[cfg(test)]
mod tests {
    use super::Writer;
    use buffers::Buffer;
    use std::sync::Arc;
    use torrent::peer::Message;

    #[test]
    fn test_write_keepalive() {
        let mut w = Writer::new();
        let mut buf = [1u8; 4];
        let m = Message::KeepAlive;
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0u8; 4])
    }

    #[test]
    fn test_write_choke() {
        let mut w = Writer::new();
        let mut buf = [0u8; 5];
        let m = Message::Choke;
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 1, 0])
    }

    #[test]
    fn test_write_unchoke() {
        let mut w = Writer::new();
        let mut buf = [0u8; 5];
        let m = Message::Unchoke;
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 1, 1])
    }

    #[test]
    fn test_write_interested() {
        let mut w = Writer::new();
        let mut buf = [0u8; 5];
        let m = Message::Interested;
        w.write_message(m, &mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 1, 2]);
        // test split write
        w.writable(&mut &mut buf[0..1]).unwrap();
        w.writable(&mut &mut buf[1..3]).unwrap();
        w.writable(&mut &mut buf[3..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 1, 2]);
    }

    #[test]
    fn test_write_have() {
        let mut w = Writer::new();
        let mut buf = [0u8; 9];
        let m = Message::Have(1);
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 5, 4, 0, 0, 0, 1])
    }

    #[test]
    fn test_write_bitfield() {
        use torrent::Bitfield;
        let mut w = Writer::new();
        let mut buf = [0u8; 9];
        let mut pf = Bitfield::new(32);
        for i in 0..32 {
            pf.set_bit(i);
        }
        let m = Message::Bitfield(pf);
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 5, 5, 0xff, 0xff, 0xff, 0xff])
    }

    #[test]
    fn test_write_request() {
        let mut w = Writer::new();
        let mut buf = [0u8; 17];
        let m = Message::Request {
            index: 1,
            begin: 1,
            length: 1,
        };
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 13, 6, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    }

    #[test]
    fn test_write_piece() {
        use std::io::Cursor;
        let mut w = Writer::new();
        let mut piece = Buffer::get().expect("buffers should be present in tests");
        for i in 0..b.len() {
            piece[i] = 1;
        }
        let mut sbuf = [0u8; 16_384 + 13];
        let mut buf = Cursor::new(&mut sbuf[..]);
        let m = Message::Piece {
            index: 1,
            begin: 1,
            length: 16_384,
            data: piece,
        };
        w.write_message(m, &mut buf).unwrap();
        let buf = buf.into_inner();
        assert_eq!(buf[0..13], [0, 0, 0x40, 0x09, 7, 0, 0, 0, 1, 0, 0, 0, 1]);
        for i in 0..16_384 {
            assert_eq!(buf[i + 13], 1);
        }
    }

    #[test]
    fn test_write_cancel() {
        let mut w = Writer::new();
        let mut buf = [0u8; 17];
        let m = Message::Cancel {
            index: 1,
            begin: 1,
            length: 1,
        };
        w.write_message(m, &mut &mut buf[..]).unwrap();
        assert_eq!(buf, [0, 0, 0, 13, 8, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
    }

    #[test]
    fn test_write_handshake() {
        use PEER_ID;
        let mut w = Writer::new();
        let m = Message::Handshake {
            rsv: [0; 8],
            hash: [0; 20],
            id: *PEER_ID,
        };
        let mut buf = [0u8; 68];
        let mut abuf = [0u8; 68];
        m.encode(&mut abuf).unwrap();
        w.write_message(m, &mut &mut buf[..]).unwrap();
        w.writable(&mut &mut buf[..]).unwrap();
        assert_eq!(buf[..], abuf[..])
    }
}
