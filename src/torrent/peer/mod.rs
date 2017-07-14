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
use control::cio;

error_chain! {
    errors {
        ProtocolError(r: &'static str) {
            description("Peer did not conform to the bittorrent protocol")
                display("Peer protocol error: {:?}", r)
        }
    }
}

/// Peer connection and associated metadata.
pub struct Peer<T: cio::CIO> {
    id: usize,
    cio: T,
    pieces: Bitfield,
    remote_status: Status,
    local_status: Status,
    queued: u16,
    tid: usize,
    downloaded: usize,
    uploaded: usize,
    addr: SocketAddr,
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

    pub fn sock(&self) -> &Socket {
        &self.sock
    }

    pub fn sock_mut(&mut self) -> &mut Socket {
        &mut self.sock
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

#[cfg(test)]
impl Peer<cio::test::TCIO> {
    pub fn test(id: usize, uploaded: usize, downloaded: usize, queued: u16, pieces: Bitfield) -> Peer<cio::test::TCIO> {
        Peer {
            id,
            remote_status: Status::new(),
            local_status: Status::new(),
            uploaded,
            downloaded,
            addr: "127.0.0.1:0".parse().unwrap(),
            cio: cio::test::TCIO::new(),
            queued,
            pieces,
            tid: 0,
        }
    }

    pub fn test_from_pieces(id: usize, pieces: Bitfield) -> Peer<cio::test::TCIO> {
        Peer::test(id, 0, 0, 0, pieces)
    }

    pub fn test_from_stats(id: usize, ul: usize, dl: usize) -> Peer<cio::test::TCIO> {
        Peer::test(id, ul, dl, 0, Bitfield::new(4))
    }
}

impl<T: cio::CIO> Peer<T> {
    pub fn new(mut conn: PeerConn, t: &mut Torrent<T>) -> cio::Result<Peer<T>> {
        let addr = conn.sock().addr();
        conn.set_throttle(t.get_throttle(0));
        let id = t.cio.add_peer(conn)?;
        let mut p = Peer {
            id,
            addr,
            remote_status: Status::new(),
            local_status: Status::new(),
            uploaded: 0,
            downloaded: 0,
            cio: t.cio.new_handle(),
            queued: 0,
            pieces: Bitfield::new(t.info.hashes.len() as u64),
            tid: t.id,
        };
        p.send_message(Message::handshake(&t.info));
        p.send_message(Message::Bitfield(t.pieces.clone()));
        Ok(p)
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn pieces(&self) -> &Bitfield {
        &self.pieces
    }

    #[cfg(test)]
    pub fn pieces_mut(&mut self) -> &mut Bitfield {
        &mut self.pieces
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn flush_ul(&mut self) -> usize {
        mem::replace(&mut self.uploaded, 0)
    }

    pub fn flush_dl(&mut self) -> usize {
        mem::replace(&mut self.downloaded, 0)
    }

    pub fn can_queue_req(&mut self) -> bool {
        !self.remote_status.choked && self.queued < 5
    }

    pub fn handle_msg(&mut self, msg: &mut Message) -> Result<()> {
        match *msg {
            Message::Piece { .. } => {
                self.downloaded += 1;
                self.queued -= 1;
            }
            Message::Request { .. } => {
                return Err(ErrorKind::ProtocolError("Peer requested while choked!").into())
            }
            Message::Choke { .. } => {
                self.remote_status.choked = true;
            }
            Message::Unchoke { .. } => {
                self.remote_status.choked = false;
            }
            Message::Interested { .. } => {
                self.remote_status.interested = true;
            }
            Message::Uninterested { .. } => {
                self.remote_status.interested = false;
            }
            Message::Have(idx) => {
                if idx >= self.pieces.len() as u32 {
                    return Err(ErrorKind::ProtocolError("Invalid piece provided in HAVE!").into())
                }
                self.pieces.set_bit(idx as u64);
            }
            Message::Bitfield(ref mut pieces) => {
                // Set the correct length, then swap the pieces
                pieces.cap(self.pieces.len());
                mem::swap(pieces, &mut self.pieces);
            }
            Message::KeepAlive => {
                // TODO: Keep track of some internal timer maybe?
            }
            Message::Cancel { .. } => {
                // TODO: Attempt to drain CIO write queue?
                /*
                self.conn.writer.write_queue.retain(|m| {
                    if let Message::Piece { index: i, begin: b, .. } = *m {
                        return !(i == index && b == begin);
                    }
                    return true;
                });
                */
            }
            _ => { }
        }
        Ok(())
    }

    pub fn request_piece(&mut self, idx: u32, offset: u32, len: u32) {
        let m = Message::request(idx, offset, len);
        self.queued += 1;
        self.send_message(m);
    }

    pub fn choke(&mut self) {
        if !self.local_status.choked {
            self.local_status.choked = true;
            self.send_message(Message::Choke);
        }
    }

    pub fn unchoke(&mut self) {
        if self.local_status.choked {
            self.local_status.choked = false;
            self.send_message(Message::Unchoke);
        }
    }

    pub fn interested(&mut self) {
        if !self.local_status.interested {
            self.local_status.interested = true;
            self.send_message(Message::Interested);
        }
    }

    pub fn uninterested(&mut self) {
        if self.local_status.interested {
            self.local_status.interested = false;
            self.send_message(Message::Uninterested);
        }
    }

    pub fn send_port(&mut self, port: u16) {
        self.send_message(Message::Port(port));
    }

    pub fn send_message(&mut self, msg: Message) {
        if msg.is_piece() {
            self.uploaded += 1;
        }
        self.cio.msg_peer(self.id, msg);
    }
}

impl<T: cio::CIO> Drop for Peer<T> {
    fn drop(&mut self) {
        self.cio.remove_peer(self.id);
    }
}

impl<T: cio::CIO> fmt::Debug for Peer<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Peer {{ id: {}, tid: {}, local_status: {:?}, remote_status: {:?} }}",
               self.id, self.tid, self.local_status, self.remote_status)
    }
}
