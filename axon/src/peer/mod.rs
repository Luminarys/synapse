mod reader;
mod writer;

use mio::tcp::TcpStream;
use self::reader::Reader;
use self::writer::Writer;
use torrent::{Torrent, TorrentStatus};
use piece_field::PieceField;
use message::Message;
use util::io_err;
use std::io;

pub enum Event {
    ReceivedPiece { piece: u32, offset: u32 },
    CompletedTorrent,
    AllowReciprocation,
    RevokeReciprocation,
}

pub struct IncomingPeer {
    conn: TcpStream,
    reader: Reader,
}

impl IncomingPeer {
    pub fn new(conn: TcpStream) -> IncomingPeer {
        IncomingPeer {
            conn: conn,
            reader: Reader::new(),
        }
    }

    pub fn readable(&mut self) -> io::Result<Option<Message>> {
        if let Some(msg) = self.reader.readable(&mut self.conn)? {
            return Ok(Some(msg));
        }
        Ok(None)
    }

    pub fn socket(&self) -> &TcpStream {
        &self.conn
    }

    fn consume(self) -> (TcpStream, Reader) {
        (self.conn, self.reader)
    }
}

pub struct Peer {
    conn: TcpStream,
    pub data: PeerData,
    pub id: Option<[u8; 20]>,
    received_bitfield: bool,
    state: State,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    /// Connection incoming from another client to us
    pub fn new_client(mut p: IncomingPeer, id: [u8; 20], torrent: &Torrent) -> Peer {
        let (mut conn, reader) = p.consume();
        let mut w = Writer::new();
        w.write_message(Message::handshake(torrent.info()), &mut conn);
        Peer {
            data: PeerData::new(torrent),
            conn: conn,
            reader: reader,
            writer: w,
            id: Some(id),
            received_bitfield: false,
            // We've already received the conn, must be valid
            state: State::Valid,
        }
    }

    /// Connection outgoing from us to another client
    pub fn new_server(mut conn: TcpStream, torrent: &Torrent) -> Peer {
        let mut w = Writer::new();
        w.write_message(Message::handshake(torrent.info()), &mut conn);
        Peer {
            data: PeerData::new(torrent),
            conn: conn,
            reader: Reader::new(),
            writer: w,
            id: None,
            received_bitfield: false,
            state: State::Initial,
        }
    }

    pub fn socket(&self) -> &TcpStream {
        &self.conn
    }

    pub fn readable(&mut self, torrent: &mut Torrent) -> io::Result<()> {
        while let Some(msg) = self.reader.readable(&mut self.conn)? {
            self.handle_msg(msg, torrent)?;
        }
        Ok(())
    }

    pub fn writable(&mut self, torrent: &mut Torrent) -> io::Result<()> {
        self.writer.writable(&mut self.conn)?;
        Ok(())
    }

    pub fn alive(&mut self, torrent: &mut Torrent) -> bool {
        unimplemented!();
    }

    pub fn handle_ev(&mut self, torrent: &mut Torrent, event: Event) {
    }

    fn handle_msg(&mut self, msg: Message, torrent: &mut Torrent) -> io::Result<()> {
        match (self.state, msg) {
            (State::Initial, Message::Handshake { rsv, hash, id }) => {
                self.state = State::Valid;
                self.id = Some(id);
            }
            (State::Initial, _) => { return io_err("Must receive handshake first!"); }
            (State::Valid, Message::Bitfield(pf)) => {
                if self.received_bitfield {
                    return io_err("Can only receive bitfield once!");
                }
                self.received_bitfield = true;
                self.data.pieces = pf;
                torrent.picker().pick(&self.data.pieces);
            }
            _ => { }
        }
        Ok(())
    }

    pub fn has_piece(&self, idx: u32) -> bool {
        return self.data.pieces.has_piece(idx);
    }
}

#[derive(Copy, Clone, Debug)]
enum State {
    // Starting state for an incomplete torrent, waiting for events
    Initial,
    // The handshake went through, the peer is valid
    Valid,
    // The peer has stuff we want, we're waiting for them to unchoke us
    AwaitingUnchoke,
    // We've been unchoked and can now download
    Unchoked,
    // We sent a request and are waiting for a piece back
    AwaitingPiece,
    // We have everything
    Seeding,
}

#[derive(Debug)]
pub enum Interest {
    Interested,
    Uninterested,
}

#[derive(Debug)]
pub enum Choke {
    Choked,
    Unchoked,
}

#[derive(Debug)]
pub struct PeerData {
    // Remote Interest
    pub interest: Interest,
    // Local choke
    pub choking: Choke,
    pub received: u32,
    pub pieces: PieceField,
    pub assigned_piece: Option<u32>,
    pub torrent: [u8; 20],
}

impl PeerData {
    fn new(torrent: &Torrent) -> PeerData {
        PeerData {
            interest: Interest::Uninterested,
            choking: Choke::Choked,
            received: 0,
            pieces: PieceField::new(torrent.status().pieces.len()),
            assigned_piece: None,
            torrent: torrent.info().hash,
        }
    }
}
