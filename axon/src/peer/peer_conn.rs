use peer::{Peer, PeerEvent, PeerResp};
use torrent::Torrent;
use message::Message;
use mio::tcp::TcpStream;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::mem;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::collections::VecDeque;
use std::time::Instant;

pub struct PeerConn {
    peer: Peer,
    conn: TcpStream,
    recv_state: RecvState,
    write_state: WriteState,
    writable: bool,
    pub readable: bool,
    write_queue: VecDeque<Message>,
}

enum RecvState {
    Idle,
    ReceivingLen { data: [u8; 4], idx: u8 },
    ReceivingMsg { data: [u8; 9], len: u8, idx: u8 },
    ReceivingPiece { prefix: [u8; 8], data: [u8; 16384], idx: u16 },
}

enum WriteState {
    Idle,
    /// len is the actual message length(could be less than 13 bytes)
    WritingMsg { data: [u8; 13], len: u8, idx: u8 },
    WritingPiece { prefix: [u8; 8], data: Arc<[u8; 16384]>, idx: u16 }
}

pub enum PeerConnResp {
    None,
    GotPiece { idx: usize, offset: usize, data: [u8; 16384] },
    WantPiece { idx: usize, offset: usize },
}

impl PeerConn {
    fn new(conn: TcpStream, torrent: &Torrent) -> PeerConn {
        PeerConn {
            peer: Peer::new(torrent.status()),
            conn: conn,
            recv_state: RecvState::Idle,
            write_state: WriteState::Idle,
            write_queue: VecDeque::new(),
            writable: false,
            readable: true,
        }
    }

    fn readable(&mut self) -> io::Result<PeerConnResp> {
        let state = mem::replace(&mut self.recv_state, RecvState::Idle);
        self.readable = false;
        let new_state = match state {
            RecvState::Idle => {
                let mut len_buf = [u8; 4];
                let amount = self.conn.read(&mut len_buf)?;
                if amount == 0 {
                    // Spurious read
                    RecvState::Idle
                } else if amount < 4 {
                    RecvState::ReceivingLen { data: len_buf, idx: amount }
                } else {
                    self.readable = true;
                    let len = len_buf.read_u32::<BigEndian>()?;
                    if len <= 9 {
                        RecvState::ReceivingMessage { data: [u8; 9], len: len, idx: 0 }
                    } else {
                        RecvState::ReceivingPiece { data: data, idx: 0 }
                    }
                }
            }
            RecvState::ReceivingLen { data, idx } => {
                let amount = self.conn.read(&mut data[(idx as usize)..])? as u8;
                if idx + amount == 4 {
                    self.readable = true;
                    let len = len_buf.read_u32::<BigEndian>()?;
                    if len <= 9 {
                        RecvState::ReceivingMessage { data: [u8; 9], len: len, idx: 0 }
                    } else {
                        RecvState::ReceivingPiece { data: data, idx: 0 }
                    }
                } else {
                    RecvState::ReceivingLen { data: len_buf, idx: idx + amount }
                }
            }
            RecvState::ReceivingMsg { data, idx } => {
            }
            RecvState::ReceivingPiece { mut data, idx } => {
                let amount = self.conn.read(&mut data[(idx as usize)..])? as u16;
                if idx + amount == data.len() as u16 {
                    self.readable = true;
                    RecvState::Idle
                } else {
                    RecvState::ReceivingPiece { data: data, idx: idx + amount }
                }
            }
        };
        self.recv_state = new_state;
        Ok(PeerConnResp::None)
    }

    fn assign_piece(&mut self, piece: u32) {
        self.peer.assign_piece(piece);
    }

    fn recieved_piece(&mut self, piece: u32) {
        self.peer.handle();
    }

    fn send_piece(&mut self, piece: Arc<[u8; 16384]>) {
    }
    
    fn writable(&mut self) {
        self.writable = true;
    }

    fn checkup(&mut self) -> Result<(), ()> {
        Ok(())
    }
}
