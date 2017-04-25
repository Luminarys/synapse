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
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            conn: conn,
            reader: Reader::new(),
            writer: writer,
        })
    }

    pub fn new_incoming(mut conn: TcpStream, torrent: &Info) -> io::Result<Peer> {
        let mut writer = Writer::new();
        writer.write_message(Message::handshake(torrent), &mut conn)?;
        Ok(Peer {
            conn: conn,
            reader: Reader::new(),
            writer: writer,
        })
    }

    fn readable(&mut self) -> io::Result<Vec<Message>> {
        return self.reader.readable(&mut self.conn);
    }

    fn writable(&mut self) -> io::Result<()> {
        self.writer.writable(&mut self.conn);
        Ok(())
    }
}
