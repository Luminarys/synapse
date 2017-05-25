use torrent::peer::Message;
use std::collections::VecDeque;
use std::io::{self, Write, ErrorKind};
use std::sync::Arc;
use util::io_err;

pub struct Writer {
    blocks_written: usize,
    writable: bool,
    write_queue: VecDeque<Message>,
    state: WriteState,
}

enum WriteState {
    Idle,
    WritingMsg { data: [u8; 17], len: u8, idx: u8 },
    WritingOther { data: Vec<u8>, idx: u16 },
    WritingPiece { prefix: [u8; 17], data: Arc<[u8; 16384]>, idx: u16 }
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

    pub fn is_writable(&self) -> bool {
        return self.writable;
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
                Message::SharedPiece{ data, .. } => {
                    WriteState::WritingPiece { prefix: buf, data: data, idx: 0 }
                }
                _ => {
                    WriteState::WritingMsg { data: buf, len: len as u8, idx: 0 }
                }
            }
        } else {
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
                Ok(false) => { }
                Err(e) => {
                    if e.kind() == ErrorKind::WouldBlock {
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
            WriteState::WritingMsg { ref data, ref len, ref mut idx } => {
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
            WriteState::WritingPiece { ref prefix, ref data, ref mut idx } => {
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
                if *idx == (prefix.len() + data.len()) as u16 {
                    self.blocks_written += 1;
                    Ok(true)
                } else {
                    self.writable = false;
                    Ok(false)
                }
            }
            WriteState::WritingOther { ref data, ref mut idx } => {
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

#[test]
fn test_write_keepalive() {
    let mut w = Writer::new();
    let mut buf = [1u8; 4];
    let m = Message::KeepAlive;
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0u8; 4])
}

#[test]
fn test_write_choke() {
    let mut w = Writer::new();
    let mut buf = [0u8; 5];
    let m = Message::Choke;
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0, 0, 0, 1, 0])
}

#[test]
fn test_write_unchoke() {
    let mut w = Writer::new();
    let mut buf = [0u8; 5];
    let m = Message::Unchoke;
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0, 0, 0, 1, 1])
}

#[test]
fn test_write_interested() {
    let mut w = Writer::new();
    let mut buf = [0u8; 5];
    let m = Message::Interested;
    w.write_message(m, &mut &mut buf[..]);
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
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0, 0, 0, 5, 4, 0, 0, 0, 1])
}

#[test]
fn test_write_bitfield() {
    use torrent::piece_field::PieceField;
    let mut w = Writer::new();
    let mut buf = [0u8; 9];
    let mut pf = PieceField::new(32);
    for i in 0..32 {
        pf.set_piece(i);
    }
    let m = Message::Bitfield(pf);
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0, 0, 0, 5, 5, 0xff, 0xff, 0xff, 0xff])
}

#[test]
fn test_write_request() {
    let mut w = Writer::new();
    let mut buf = [0u8; 17];
    let m = Message::Request { index: 1, begin: 1, length: 1 };
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf, [0, 0, 0, 13, 6, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
}

#[test]
fn test_write_piece() {
    use std::io::Cursor;
    let mut w = Writer::new();
    let piece = Arc::new([1u8; 16384]);
    let mut sbuf = [0u8; 16384 + 13];
    let mut buf = Cursor::new(&mut sbuf[..]);
    let m = Message::SharedPiece { index: 1, begin: 1, data: piece };
    w.write_message(m, &mut buf);
    let buf = buf.into_inner();
    assert_eq!(buf[0..13], [0, 0, 0x40, 0x09, 7, 0, 0, 0, 1, 0, 0, 0, 1]);
    for i in 0..16384 {
        assert_eq!(buf[i + 13], 1);
    }
}

#[test]
fn test_write_cancel() {
    let mut w = Writer::new();
    let mut buf = [0u8; 17];
    let m = Message::Cancel { index: 1, begin: 1, length: 1 };
    w.write_message(m, &mut &mut buf[..]);
    assert_eq!(buf, [0, 0, 0, 13, 8, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1])
}

#[test]
fn test_write_handshake() {
    use ::PEER_ID;
    let mut w = Writer::new();
    let m = Message::Handshake { rsv: [0; 8], hash: [0; 20], id: *PEER_ID };
    let mut buf = [0u8; 68];
    let mut abuf = [0u8; 68];
    m.encode(&mut abuf);
    w.write_message(m, &mut &mut buf[..]);
    w.writable(&mut &mut buf[..]).unwrap();
    assert_eq!(buf[..], abuf[..])
}
