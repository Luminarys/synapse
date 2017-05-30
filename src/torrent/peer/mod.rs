mod reader;
mod writer;
mod message;

pub use self::message::Message;
use self::reader::Reader;
use self::writer::Writer;
use std::net::TcpStream;
use socket::Socket;
use std::net::SocketAddr;
use std::io;
use torrent::{Torrent, PieceField};

pub struct Peer {
    pub conn: Socket,
    pub pieces: PieceField,
    pub being_choked: bool,
    pub choked: bool,
    pub interested: bool,
    pub queued: u16,
    pub tid: usize,
    pub id: usize,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    fn new (conn: Socket) -> Peer {
        let writer = Writer::new();
        let reader = Reader::new();
        Peer {
            being_choked: true,
            choked: true,
            interested: false,
            conn,
            reader: reader,
            writer: writer,
            queued: 0,
            pieces: PieceField::new(8),
            tid: 0,
            id: 0,
        }
    }

    /// Creates a new peer for a torrent which will connect to another client
    pub fn new_outgoing(ip: &SocketAddr) -> io::Result<Peer> {
        Ok(Peer::new(Socket::new(ip)?))
    }

    /// Creates a peer for an unidentified incoming peer.
    /// Note that set_torrent will need to be called once the handshake is
    /// processed.
    pub fn new_incoming(conn: TcpStream) -> io::Result<Peer> {
        Ok(Peer::new(Socket::from_stream(conn)?))
    }

    /// Sets the peer's metadata to the given torrent info and sends a
    /// handshake and bitfield.
    pub fn set_torrent(&mut self, t: &Torrent) -> io::Result<()> {
        self.writer.write_message(Message::handshake(&t.info), &mut self.conn)?;
        self.pieces = PieceField::new(t.info.hashes.len() as u32);
        self.tid = t.id;
        self.writer.write_message(Message::Bitfield(t.pieces.clone()), &mut self.conn)?;
        Ok(())
    }

    pub fn readable(&mut self) -> io::Result<Vec<Message>> {
        let mut msgs = Vec::with_capacity(1);
        loop {
            if let Some(msg) = self.reader.readable(&mut self.conn)? {
                msgs.push(msg);
            } else {
                break;
            }
        }
        Ok(msgs)
    }

    pub fn read(&mut self) -> io::Result<Option<Message>> {
        return self.reader.readable(&mut self.conn);
    }

    /// Returns a boolean indicating whether or not the
    /// socket should be re-registered
    pub fn writable(&mut self) -> io::Result<bool> {
        self.writer.writable(&mut self.conn)?;
        Ok(!self.writer.is_writable())
    }

    pub fn send_message(&mut self, msg: Message) -> io::Result<()> {
        return self.writer.write_message(msg, &mut self.conn);
    }
}
