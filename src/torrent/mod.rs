pub mod info;
pub mod peer;
pub mod tracker;
mod picker;

pub mod piece_field;

pub use self::piece_field::PieceField;
pub use self::info::Info;
pub use self::peer::Peer;

use bencode::BEncode;
use self::peer::Message;
use self::picker::Picker;
use slab::Slab;
use std::{fmt, io};
use disk;
use std::sync::mpsc;
use pbr::ProgressBar;

pub struct Torrent {
    pub info: Info,
    pub pieces: PieceField,
    peers: Slab<Peer, usize>,
    picker: Picker,
    disk: mpsc::Sender<disk::Request>,
    pb: ProgressBar<io::Stdout>,
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
        println!("Handling with torrent with {:?} pl, {:?} pieces, {:?} sf len", info.piece_len, info.pieces(), info.files.last().unwrap().length);
        // Create dummy files
        info.create_files().unwrap();
        let peers = Slab::with_capacity(32);
        let pieces = PieceField::new(info.pieces());
        let picker = Picker::new(&info);
        let pb = ProgressBar::new(info.pieces() as u64);
        let disk = ::DISK.get();
        // let tracker = Tracker::new().unwrap();
        Ok(Torrent { info, peers, pieces, picker, disk, pb })
    }

    pub fn peer_readable(&mut self, peer: usize) -> io::Result<()> {
        let res = self.peers.get_mut(peer).unwrap().readable()?;
        for msg in res {
            self.handle_msg(msg, peer)?;
        }
        Ok(())
    }

    fn handle_msg(&mut self, msg: Message, peer: usize) -> io::Result<()> {
        let peer = self.peers.get_mut(peer).unwrap();
        match msg {
            Message::Bitfield(mut pf) => {
                pf.cap(self.pieces.len());
                peer.pieces = pf;
                if self.pieces.usable(&peer.pieces) {
                    peer.send_message(Message::Interested)?;
                }
            }
            Message::Have(idx) => {
                peer.pieces.set_piece(idx);
            }
            Message::Unchoke => {
                peer.being_choked = false;
                Torrent::make_requests(&mut self.picker, peer, &self.info)?;
            }
            Message::Choke => {
                peer.being_choked = true;
            }
            Message::Piece { index, begin, data } => {
                peer.queued -= 1;
                let len = if !self.info.is_last_piece((index, begin)) {
                    16384
                } else {
                    self.info.last_piece_len()
                };
                Torrent::write_piece(&self.info, index, begin, len, data, &self.disk);
                if self.picker.completed(index, begin) {
                    self.pb.inc();
                    self.pieces.set_piece(index);
                    if self.pieces.complete() {
                        self.pb.finish_print("Downloaded!");
                    }
                    // Broadcast HAVE to everyone who needs it.
                }
                if !peer.being_choked {
                    Torrent::make_requests(&mut self.picker, peer, &self.info)?;
                }
            }
            _ => { }
        }
        Ok(())
    }

    #[inline(always)]
    fn write_piece(info: &Info, index: u32, begin: u32, len: u32, data: Box<[u8; 16384]>, disk: &mpsc::Sender<disk::Request>) {
        let mut idx = 0;
        let mut fidx = 0;
        for _ in 0..index {
            idx += info.piece_len;
            if idx > info.files[fidx].length {
                idx -= info.files[fidx].length;
                fidx += 1;
            }
        }
        // TODO: Handle the multi file boundary!
        let offset = idx as u64 + begin as u64;
        let file = info.files[fidx].path.clone();
        let req = disk::Request { file, data, offset, start: 0, end: len as usize };
        disk.send(req).unwrap();
    }

    #[inline(always)]
    fn make_requests(picker: &mut Picker, peer: &mut Peer, info: &Info) -> io::Result<()> {
        // keep 5 outstanding reuqests?
        while peer.queued < 5 {
            if let Some((idx, offset)) = picker.pick(&peer) {
                if info.is_last_piece((idx, offset)) {
                    peer.send_message(Message::request(idx, offset, info.last_piece_len()))?;
                } else {
                    peer.send_message(Message::request(idx, offset, 16384))?;
                }
                peer.queued += 1;
            } else {
                break;
            }
        }
        Ok(())
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
