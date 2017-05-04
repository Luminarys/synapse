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
    pub queued: u16,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    pub fn new_outgoing(ip: &SocketAddr, torrent: &Info) -> io::Result<Peer> {
        let mut conn = Socket::new(TcpStream::connect(ip)?);
        let mut writer = Writer::new();
        let mut reader = Reader::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            being_choked: true,
            conn,
            reader,
            writer,
            queued: 0,
            pieces: PieceField::new(torrent.hashes.len() as u32),
        })
    }

    pub fn new_incoming(mut conn: TcpStream, torrent: &Info) -> io::Result<Peer> {
        let mut writer = Writer::new();
        let mut reader = Reader::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            being_choked: true,
            conn: Socket::new(conn),
            reader: reader,
            writer: writer,
            queued: 0,
            pieces: PieceField::new(torrent.hashes.len() as u32),
        })
    }

    pub fn readable(&mut self) -> io::Result<Vec<Message>> {
        return self.reader.readable(&mut self.conn);
    }

    /// Returns a boolean indicating whether or not the
    /// socket should be re-registered
    pub fn writable(&mut self) -> io::Result<bool> {
        self.writer.writable(&mut self.conn);
        Ok(!self.writer.is_writable())
    }

    pub fn send_message(&mut self, msg: Message) -> io::Result<()> {
        return self.writer.write_message(msg, &mut self.conn);
    }
}
