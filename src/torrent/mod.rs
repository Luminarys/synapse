pub mod info;
pub mod peer;
pub mod tracker;
mod picker;

pub mod piece_field;

pub use self::piece_field::PieceField;
pub use self::info::Info;
pub use self::peer::Peer;

use bencode::BEncode;
use self::tracker::Tracker;
use self::peer::Message;
use self::picker::Picker;
use slab::Slab;
use std::{fmt, io};
use mio::Poll;

pub struct Torrent {
    pub info: Info,
    pub pieces: PieceField,
    peers: Slab<Peer, usize>,
    picker: Picker,
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
        let picker = Picker::new(&info);
        // let tracker = Tracker::new().unwrap();
        Ok(Torrent { info, peers, pieces, picker })
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
            Message::Bitfield(pf) => {
                println!("Assigning pf, len {:?}, ours {:?}!", pf.len(), self.pieces.len());
                peer.pieces = pf;
                if self.pieces.usable(&peer.pieces) {
                    println!("Peer is interesting!");
                    peer.send_message(Message::Interested);
                }
            }
            Message::Have(idx) => {
                println!("Setting have for peer!");
                peer.pieces.set_piece(idx);
            }
            Message::Unchoke => {
                println!("Unchoked, attempting request!");
                peer.being_choked = false;
                Torrent::make_requests(&mut self.picker, peer);
            }
            Message::Choke => {
                peer.being_choked = true;
            }
            Message::Piece { index, begin, data } => {
                peer.queued -= 1;
                println!("Piece {:?}, {:?} received!", index, begin);
                if self.picker.completed(index, begin) {
                    self.pieces.set_piece(index);
                    // Broadcast HAVE to everyone who needs it.
                }
                if !peer.being_choked {
                    Torrent::make_requests(&mut self.picker, peer);
                }
            }
            _ => { }
        }
    }

    fn make_requests(picker: &mut Picker, peer: &mut Peer) {
        while peer.queued < 10 {
            if let Some((idx, offset)) = picker.pick(&peer) {
                println!("Requesting {:?}, {:?}!", idx, offset);
                peer.send_message(Message::request(idx, offset));
                peer.queued += 1;
            } else {
                break;
            }
        }
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
