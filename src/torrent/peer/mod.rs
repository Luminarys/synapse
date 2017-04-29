mod reader;
mod writer;
mod message;

use self::reader::Reader;
use self::writer::Writer;
use self::message::Message;
use mio::tcp::TcpStream;
use std::net::SocketAddr;
use std::io;
use torrent::info::Info;

pub struct Peer {
    pub conn: TcpStream,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    pub fn new_outgoing(ip: &SocketAddr, torrent: &Info) -> io::Result<Peer> {
        let mut conn = TcpStream::connect(ip)?;
        let mut writer = Writer::new();
        let mut reader = Reader::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            conn: conn,
            reader: reader,
            writer: writer,
        })
    }

    pub fn new_incoming(mut conn: TcpStream, torrent: &Info) -> io::Result<Peer> {
        let mut writer = Writer::new();
        let mut reader = Reader::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            conn: conn,
            reader: reader,
            writer: writer,
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
}
