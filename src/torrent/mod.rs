pub mod info;
pub mod peer;
mod picker;

pub mod piece_field;

pub use self::piece_field::PieceField;
pub use self::info::Info;
pub use self::peer::Peer;

use bencode::BEncode;
use self::peer::Message;
use self::picker::Picker;
use std::{fmt, io, cmp};
use {disk, DISK};
use pbr::ProgressBar;
use std::collections::HashSet;

pub struct Torrent {
    pub info: Info,
    pub pieces: PieceField,
    pub uploaded: usize,
    pub downloaded: usize,
    pub id: usize,
    peers: HashSet<usize>,
    picker: Picker,
    pb: ProgressBar<io::Stdout>,
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
        let peers = HashSet::new();
        let pieces = PieceField::new(info.pieces());
        let picker = Picker::new(&info);
        let pb = ProgressBar::new(info.pieces() as u64);
        Ok(Torrent { id: 0, info, peers, pieces, picker, pb, uploaded: 0, downloaded: 0 })
    }

    pub fn block_available(&mut self, peer: &mut Peer, resp: disk::Response) -> io::Result<()> {
        let ctx = resp.context;
        let p = Message::s_piece(ctx.idx, ctx.begin, ctx.length, resp.data);
        peer.send_message(p)?;
        Ok(())
    }

    pub fn peer_readable(&mut self, peer: &mut Peer) -> io::Result<()> {
        let res = peer.readable()?;
        for msg in res {
            self.handle_msg(msg, peer)?;
        }
        Ok(())
    }

    pub fn handle_msg(&mut self, msg: Message, peer: &mut Peer) -> io::Result<()> {
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
            Message::Piece { index, begin, data, length } => {
                peer.queued -= 1;
                Torrent::write_piece(&self.info, index, begin, length, data);
                if self.picker.completed(index, begin) {
                    self.pb.inc();
                    self.pieces.set_piece(index);
                    if self.pieces.complete() {
                        self.pb.finish_print("Downloaded!");
                    }
                    // TODO: Broadcast HAVE to everyone who needs it.
                }
                if !peer.being_choked {
                    Torrent::make_requests(&mut self.picker, peer, &self.info)?;
                }
            }
            Message::Request { index, begin, length } => {
                // TODO get this from some sort of allocator.
                Torrent::request_read(peer.id, &self.info, index, begin, length, Box::new([0u8; 16384]));
            }
            Message::Interested => {
                peer.interested = true;
                // If we're in seed mode, upload, otherwise
                // use the general tit for tat heuristic for unchoking.
                // TODO: implement said heuristic
                if self.pieces.complete() {
                    peer.choked = false;
                    peer.send_message(Message::Unchoke)?;
                }
            }
            Message::Uninterested => {
                peer.interested = false;
                if !peer.choked {
                    peer.choked = true;
                    peer.send_message(Message::Choke)?;
                }
            }
            _ => { }
        }
        Ok(())
    }

    /// Calculates the file offsets for a given index, begin, and block length.
    fn calc_block_locs(info: &Info, index: u32, begin: u32, mut len: u32) -> Vec<disk::Location> {
        // The absolute byte offset where we start processing data.
        let mut cur_start = index * info.piece_len as u32 + begin;
        // Current index of the data block we're writing
        let mut data_start = 0;
        // The current file end length.
        let mut fidx = 0;
        // Iterate over all file lengths, if we find any which end a bigger
        // idx than cur_start, write from cur_start..cur_start + file_write_len for that file
        // and continue if we're now at the end of the file.
        let mut locs = Vec::new();
        for f in info.files.iter() {
            fidx += f.length;
            if (cur_start as usize) < fidx {
                let file_write_len = cmp::min(fidx - cur_start as usize, len as usize);
                let offset = (cur_start - (fidx - f.length) as u32) as u64;
                if file_write_len == len as usize {
                    // The file is longer than our len, just write to it,
                    // exit loop
                    locs.push(disk::Location::new(f.path.clone(), offset, data_start, data_start + file_write_len));
                    break;
                } else {
                    // Write to the end of file, continue
                    locs.push(disk::Location::new(f.path.clone(), offset, data_start, data_start + file_write_len as usize));
                    len -= file_write_len as u32;
                    cur_start += file_write_len as u32;
                    data_start += file_write_len;
                }
            }
        }
        locs
    }

    #[inline(always)]
    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(info: &Info, index: u32, begin: u32, len: u32, data: Box<[u8; 16384]>) {
        let locs = Torrent::calc_block_locs(info, index, begin, len);
        DISK.tx.send(disk::Request::write(data, locs)).unwrap();
    }

    #[inline(always)]
    /// Issues a read request of the given torrent
    fn request_read(id: usize, info: &Info, index: u32, begin: u32, len: u32, data: Box<[u8; 16384]>) {
        let locs = Torrent::calc_block_locs(info, index, begin, len);
        let ctx = disk::Ctx::new(id, index, begin, len);
        DISK.tx.send(disk::Request::read(ctx, data, locs)).unwrap();
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

    pub fn peer_writable(&mut self, peer: &mut Peer) -> io::Result<bool> {
        peer.writable()
    }

    pub fn file_size(&self) -> usize {
        let mut size = 0;
        for file in self.info.files.iter() {
            size += file.length;
        }
        size
    }

    pub fn remove_peer(&mut self, id: &usize) {
        self.peers.remove(id);
    }

    pub fn insert_peer(&mut self, id: usize) {
        self.peers.insert(id);
    }
}
