mod reader;
mod writer;
mod message;

pub use self::message::Message;
use self::reader::Reader;
use self::writer::Writer;
use std::net::TcpStream;
use socket::Socket;
use std::net::SocketAddr;
use std::{io, fmt};
use torrent::{Torrent, Bitfield};

/// Peer connection and associated metadata.
pub struct Peer {
    pub conn: Socket,
    pub pieces: Bitfield,
    pub being_choked: bool,
    pub choked: bool,
    pub interested: bool,
    pub interesting: bool,
    pub queued: u16,
    pub tid: usize,
    pub id: usize,
    pub downloaded: usize,
    pub uploaded: usize,
    error: Option<io::Error>,
    reader: Reader,
    writer: Writer,
}

impl Peer {
    pub fn new (conn: Socket) -> Peer {
        let writer = Writer::new();
        let reader = Reader::new();
        Peer {
            being_choked: true,
            choked: true,
            interested: false,
            interesting: false,
            uploaded: 0,
            downloaded: 0,
            conn,
            reader: reader,
            writer: writer,
            queued: 0,
            pieces: Bitfield::new(0),
            error: None,
            tid: 0,
            id: 0,
        }
    }

    /// Creates a new "outgoing" peer, which acts as a client.
    /// Once created, set_torrent should be called.
    pub fn new_outgoing(ip: &SocketAddr) -> io::Result<Peer> {
        Ok(Peer::new(Socket::new(ip)?))
    }

    /// Creates a peer where we are acting as the server.
    /// Once the handshake is received, set_torrent should be called.
    pub fn new_incoming(conn: TcpStream) -> io::Result<Peer> {
        Ok(Peer::new(Socket::from_stream(conn)?))
    }

    pub fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }

    pub fn set_error(&mut self, err: io::Error) {
        self.error = Some(err);
    }

    /// Sets the peer's metadata to the given torrent info and sends a
    /// handshake and bitfield.
    pub fn set_torrent(&mut self, t: &Torrent) {
        if let Err(e) = self._set_torrent(t) {
            self.error = Some(e);
        }
    }

    fn _set_torrent(&mut self, t: &Torrent) -> io::Result<()> {
        self.writer.write_message(Message::handshake(&t.info), &mut self.conn)?;
        self.pieces = Bitfield::new(t.info.hashes.len() as u64);
        self.tid = t.id;
        self.writer.write_message(Message::Bitfield(t.pieces.clone()), &mut self.conn)?;
        let mut throt = t.throttle.clone();
        throt.id = self.id;
        self.conn.throttle = Some(throt);
        Ok(())
    }

    /// Attempts to read as many messages as possible from
    /// the connection, returning a vector of the results.
    pub fn readable(&mut self) -> Vec<Message> {
        match self._readable() {
            Ok(r) => r,
            Err(e) => {
                self.error = Some(e);
                Vec::new()
            }
        }
    }

    fn _readable(&mut self) -> io::Result<Vec<Message>> {
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

    /// Attempts to read a single message from the peer
    pub fn read(&mut self) -> Option<Message> {
        match self._read() {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    fn _read(&mut self) -> io::Result<Option<Message>> {
        let res = self.reader.readable(&mut self.conn)?;
        if res.as_ref().map(|m| m.is_piece()).unwrap_or(false) {
            self.downloaded += 1;
        }
        Ok(res)
    }

    /// Returns a boolean indicating whether or not the
    /// socket should be re-registered
    pub fn writable(&mut self) {
        if let Err(e) = self._writable() {
            self.error = Some(e);
        }
    }

    fn _writable(&mut self) -> io::Result<()> {
        self.writer.writable(&mut self.conn)
    }

    /// Sends a message to the peer.
    pub fn send_message(&mut self, msg: Message) {
        if let Err(e) = self._send_message(msg) {
            self.error = Some(e);
        }
    }

    fn _send_message(&mut self, msg: Message) -> io::Result<()> {
        // NOTE: This is preemptive but shouldn't be substantially wrong
        if msg.is_piece() {
            self.uploaded += 1;
        }
        return self.writer.write_message(msg, &mut self.conn);
    }

    pub fn choke(&mut self) {
        if let Err(e) = self._choke() {
            self.error = Some(e);
        }
    }

    fn _choke(&mut self) -> io::Result<()> {
        if !self.choked {
            self.choked = true;
            self._send_message(Message::Choke)
        } else {
            Ok(())
        }
    }

    pub fn unchoke(&mut self) {
        if let Err(e) = self._unchoke() {
            self.error = Some(e);
        }
    }

    fn _unchoke(&mut self) -> io::Result<()> {
        if self.choked {
            self.choked = false;
            self._send_message(Message::Unchoke)
        } else {
            Ok(())
        }
    }
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Peer {{ id: {}, tid: {}, choking: {}, being_choked: {}, interested: {}, interesting: {}}}", self.id, self.tid, self.choked, self.being_choked, self.interested, self.interesting)
    }
}
