mod reader;
mod writer;

use mio::tcp::TcpStream;
use self::reader::Reader;
use self::writer::Writer;
use torrent::{Torrent, TorrentStatus};
use piece_field::PieceField;
use message::Message;
use std::io;

pub struct Peer {
    conn: TcpStream,
    data: PeerData,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    pub fn new(conn: TcpStream, tdata: &TorrentStatus) -> Peer {
        Peer {
            data: PeerData::new(tdata.pieces.len()),
            conn: conn,
            reader: Reader::new(),
            writer: Writer::new(),
        }
    }

    pub fn readable(&mut self) -> Result<(), ()> {
        // while let Some(msg) = self.reader.readable().map(|_| ())? {
        //     self.handle_msg(msg);
        // }
        Ok(())
    }

    pub fn writable(&mut self) -> io::Result<()> {
        self.writer.writable(&mut self.conn)?;
        Ok(())
    }

    pub fn alive(&mut self) -> bool {
        unimplemented!();
    }

    fn handle_msg(&mut self, msg: Message) {
        unimplemented!();
    }
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
}

impl PeerData {
    fn new(pieces: u32) -> PeerData {
        PeerData {
            interest: Interest::Uninterested,
            choking: Choke::Choked,
            received: 0,
            pieces: PieceField::new(pieces),
            assigned_piece: None,
        }
    }
}
