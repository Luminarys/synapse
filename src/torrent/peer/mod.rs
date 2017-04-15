mod reader;
mod writer;
mod message;

use self::reader::Reader;
use self::writer::Writer;
use self::message::Message;
use mio::tcp::TcpStream;
use std::io;

pub struct Peer {
    conn: TcpStream,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    fn readable(&mut self) -> io::Result<()> {
        self.reader.readable(&mut self.conn);
        Ok(())
    }

    fn writable(&mut self) -> io::Result<()> {
        self.writer.writable(&mut self.conn);
        Ok(())
    }
}
