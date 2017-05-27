mod reader;
mod writer;
mod message;

pub use self::message::Message;
use self::reader::Reader;
use self::writer::Writer;
use mio::tcp::TcpStream;
use socket::Socket;
use std::net::SocketAddr;
use std::io;
use torrent::info::Info;
use torrent::PieceField;

pub struct Peer {
    pub conn: Socket,
    pub pieces: PieceField,
    pub being_choked: bool,
    pub choked: bool,
    pub interested: bool,
    pub queued: u16,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    /// Creates a new peer for a torrent which will connect to another client
    pub fn new_outgoing(ip: &SocketAddr, torrent: &Info) -> io::Result<Peer> {
        let mut conn = Socket::new(TcpStream::connect(ip)?);
        let mut writer = Writer::new();
        let reader = Reader::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            being_choked: true,
            choked: true,
            interested: false,
            conn,
            reader,
            writer,
            queued: 0,
            pieces: PieceField::new(torrent.hashes.len() as u32),
        })
    }

    /// Creates a peer for an unidentified incoming peer.
    /// Note that set_torrent will need to be called once the handshake is
    /// processed.
    pub fn new_incoming(conn: TcpStream) -> io::Result<Peer> {
        let writer = Writer::new();
        let reader = Reader::new();
        Ok(Peer {
            being_choked: true,
            choked: true,
            interested: false,
            conn: Socket::new(conn),
            reader: reader,
            writer: writer,
            queued: 0,
            pieces: PieceField::new(8),
        })
    }

    /// Sets the peer's metadata to the given torrent info and sends a
    /// handshake.
    pub fn set_torrent(&mut self, torrent: &Info) -> io::Result<()> {
        self.writer.write_message(Message::handshake(torrent), &mut self.conn)?;
        self.pieces = PieceField::new(torrent.hashes.len() as u32);
        Ok(())
    }

    pub fn readable(&mut self) -> io::Result<Vec<Message>> {
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
