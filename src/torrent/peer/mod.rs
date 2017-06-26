mod reader;
mod writer;
mod message;

pub use self::message::Message;
use self::reader::Reader;
use self::writer::Writer;
use std::net::TcpStream;
use socket::Socket;
use std::net::SocketAddr;
use std::{io, fmt, mem};
use torrent::{Torrent, Bitfield};
use throttle::Throttle;

/// Peer connection and associated metadata.
pub struct Peer {
    pieces: Bitfield,
    remote_status: Status,
    queued: u16,
    conn: PeerConn,
    id: usize,
    tid: usize,
    local_status: Status,
    downloaded: usize,
    uploaded: usize,
    error: Option<io::Error>,
}

pub struct PeerConn {
    sock: Socket,
    reader: Reader,
    writer: Writer,
}

impl PeerConn {
    pub fn new (sock: Socket) -> PeerConn {
        let writer = Writer::new();
        let reader = Reader::new();
        PeerConn {
            sock,
            writer,
            reader,
        }
    }

    #[cfg(test)]
    pub fn test() -> PeerConn {
        PeerConn::new(Socket::empty())
    }

    pub fn sock(&self) -> &Socket {
        &self.sock
    }

    /// Creates a new "outgoing" peer, which acts as a client.
    /// Once created, set_torrent should be called.
    pub fn new_outgoing(ip: &SocketAddr) -> io::Result<PeerConn> {
        Ok(PeerConn::new(Socket::new(ip)?))
    }

    /// Creates a peer where we are acting as the server.
    /// Once the handshake is received, set_torrent should be called.
    pub fn new_incoming(sock: TcpStream) -> io::Result<PeerConn> {
        Ok(PeerConn::new(Socket::from_stream(sock)?))
    }

    pub fn writable(&mut self) -> io::Result<()> {
        self.writer.writable(&mut self.sock)
    }

    pub fn readable(&mut self) -> io::Result<Option<Message>> {
        self.reader.readable(&mut self.sock)
    }

    pub fn write_message(&mut self, msg: Message) -> io::Result<()> {
        self.writer.write_message(msg, &mut self.sock)
    }

    pub fn set_throttle(&mut self, throt: Throttle) {
        self.sock.throttle = Some(throt);
    }
}

#[derive(Debug)]
pub struct Status {
    pub choked: bool,
    pub interested: bool,
}

impl Status {
    fn new() -> Status {
        Status { choked: true, interested: false }
    }
}

impl Peer {
    pub fn new(id: usize, mut conn: PeerConn, t: &Torrent) -> Peer {
        conn.set_throttle(t.get_throttle(id));
        let mut p = Peer {
            remote_status: Status::new(),
            local_status: Status::new(),
            uploaded: 0,
            downloaded: 0,
            conn,
            queued: 0,
            pieces: Bitfield::new(t.info.hashes.len() as u64),
            error: None,
            tid: t.id,
            id,
        };
        p.send_message(Message::handshake(&t.info));
        p.send_message(Message::Bitfield(t.pieces.clone()));
        p
    }

    #[cfg(test)]
    pub fn test(id: usize, uploaded: usize, downloaded: usize, queued: u16, pieces: Bitfield) -> Peer {
        Peer {
            id,
            remote_status: Status::new(),
            local_status: Status::new(),
            uploaded,
            downloaded,
            conn: PeerConn::test(),
            queued,
            pieces,
            error: None,
            tid: 0,
        }
    }

    #[cfg(test)]
    pub fn test_from_pieces(id: usize, pieces: Bitfield) -> Peer {
        Peer::test(id, 0, 0, 0, pieces)
    }

    #[cfg(test)]
    pub fn test_from_stats(id: usize, ul: usize, dl: usize) -> Peer {
        Peer::test(id, ul, dl, 0, Bitfield::new(4))
    }

    pub fn error(&self) -> Option<&io::Error> {
        self.error.as_ref()
    }

    pub fn set_error(&mut self, err: io::Error) {
        self.error = Some(err);
    }

    pub fn conn(&self) -> &PeerConn {
        &self.conn
    }

    pub fn pieces(&self) -> &Bitfield {
        &self.pieces
    }

    pub fn set_pieces(&mut self, bf: Bitfield) {
        self.pieces = bf;
    }

    #[cfg(test)]
    pub fn pieces_mut(&mut self) -> &mut Bitfield {
        &mut self.pieces
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn torrent(&self) -> usize {
        self.tid
    }

    pub fn local_status(&self) -> &Status {
        &self.local_status
    }

    pub fn remote_status(&self) -> &Status {
        &self.remote_status
    }

    pub fn flush_ul(&mut self) -> usize {
        mem::replace(&mut self.uploaded, 0)
    }

    pub fn flush_dl(&mut self) -> usize {
        mem::replace(&mut self.downloaded, 0)
    }

    pub fn can_queue_req(&mut self) -> bool {
        self.queued < 5
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
        while let Some(msg) = self.conn.readable()? {
            msgs.push(msg);
        }
        Ok(msgs)
    }

    /// Attempts to read a single message from the peer,
    /// processing it interally first before passing it to
    /// the torrent
    pub fn read(&mut self) -> Option<Message> {
        match self.conn.readable() {
            Ok(res) => {
                match res.as_ref() {
                    Some(&Message::Piece { .. }) => {
                        self.downloaded += 1;
                        self.queued -= 1;
                    }
                    Some(&Message::Choke { .. }) => {
                        self.remote_status.choked = true;
                    }
                    Some(&Message::Unchoke { .. }) => {
                        self.remote_status.choked = false;
                    }
                    Some(&Message::Interested { .. }) => {
                        self.remote_status.interested = true;
                    }
                    Some(&Message::Uninterested { .. }) => {
                        self.remote_status.interested = false;
                    }
                    Some(&Message::Have(idx)) => {
                        self.pieces.set_bit(idx as u64);
                    }
                    _ => { }
                }
                res
            },
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    /// Returns a boolean indicating whether or not the
    /// socket should be re-registered
    pub fn writable(&mut self) {
        if let Err(e) = self.conn.writable() {
            self.error = Some(e);
        }
    }

    /// Sends a message to the peer.
    pub fn send_message(&mut self, msg: Message) {
        if msg.is_piece() {
            self.uploaded += 1;
        }
        if let Err(e) = self.conn.write_message(msg) {
            self.error = Some(e);
        }
    }

    pub fn request_piece(&mut self, idx: u32, offset: u32, len: u32) {
        let m = Message::request(idx, offset, len);
        self.queued += 1;
        self.send_message(m);
    }

    pub fn choke(&mut self) {
        if !self.local_status.choked {
            self.local_status.choked = true;
            if let Err(e) = self.conn.write_message(Message::Choke) {
                self.error = Some(e);
            }
        }
    }

    pub fn unchoke(&mut self) {
        if self.local_status.choked {
            self.local_status.choked = false;
            if let Err(e) = self.conn.write_message(Message::Unchoke) {
                self.error = Some(e);
            }
        }
    }

    pub fn interested(&mut self) {
        if !self.local_status.interested {
            self.local_status.interested = true;
            if let Err(e) = self.conn.write_message(Message::Interested) {
                self.error = Some(e);
            }
        }
    }

    pub fn uninterested(&mut self) {
        if self.local_status.interested {
            self.local_status.interested = false;
            if let Err(e) = self.conn.write_message(Message::Uninterested) {
                self.error = Some(e);
            }
        }
    }
}

impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Peer {{ id: {}, tid: {}, local_status: {:?}, remote_status: {:?} }}",
               self.id, self.tid, self.local_status, self.remote_status)
    }
}
