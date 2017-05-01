pub mod info;
pub mod peer;
pub mod tracker;
pub mod piece_field;

pub use self::piece_field::PieceField;
pub use self::info::Info;
use bencode::BEncode;
use self::peer::Peer;
use self::tracker::Tracker;
use self::peer::Message;
use slab::Slab;
use std::{fmt, io};
use mio::Poll;

pub struct Torrent {
    pub info: Info,
    pub pieces: PieceField,
    peers: Slab<Peer, usize>,
    // tracker: Tracker,
}

impl fmt::Debug for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {{ info: {:?} }}", self.info)
    }
}

impl Torrent {
    pub fn from_bencode(data: BEncode) -> Result<Torrent, &'static str> {
        let info = Info::from_bencode(data)?;
        let peers = Slab::with_capacity(32);
        let pieces = PieceField::new(info.hashes.len() as u32);
        // let tracker = Tracker::new().unwrap();
        Ok(Torrent { info, peers, pieces })
    }

    pub fn peer_readable(&mut self, peer: usize) -> io::Result<()> {
        let res = self.peers.get_mut(peer).unwrap().readable()?;
        for msg in res {
            self.handle_msg(msg, peer);
        }
        Ok(())
    }

    fn handle_msg(&mut self, msg: Message, peer: usize) {
        let peer = self.peers.get_mut(peer).unwrap();
        match msg {
            Message::Unchoke => {
                peer.being_choked = false;
            }
            Message::Choke => {
                peer.being_choked = true;
            }
            Message::Piece { index, begin, data } => {
            
            }
            _ => { }
        }
    }

    fn request_next_piece(&mut self) {
        let m = Message::Request { index: 1, begin: 1, length: 16384 };
    }

    pub fn peer_writable(&mut self, peer: usize) -> io::Result<bool> {
        self.peers.get_mut(peer).unwrap().writable()
    }

    pub fn file_size(&self) -> usize {
        let mut size = 0;
        for file in self.info.files.iter() {
            size += file.length;
        }
        size
    }

    pub fn remove_peer(&mut self, id: usize) {
        self.peers.remove(id);
    }

    pub fn insert_peer(&mut self, peer: Peer) -> Option<usize> {
        self.peers.insert(peer).ok()
    }

    pub fn get_peer(&self, id: usize) -> Option<&Peer> {
        self.peers.get(id)
    }

    pub fn get_peer_mut(&mut self, id: usize) -> Option<&mut Peer> {
        self.peers.get_mut(id)
    }
}
