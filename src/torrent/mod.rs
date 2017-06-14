pub mod info;
pub mod peer;
pub mod bitfield;
mod picker;

pub use self::bitfield::Bitfield;
pub use self::info::Info;
pub use self::peer::Peer;

use std;
use self::peer::Message;
use self::picker::Picker;
use std::{fmt, io, cmp};
use {amy, rpc, disk, DISK, tracker, TRACKER};
use pbr::ProgressBar;
use throttle::Throttle;
use tracker::{TrackerError, TrackerRes};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use util::{io_err, random_sample};
use std::cell::UnsafeCell;

pub struct Torrent {
    pub info: Info,
    pub pieces: Bitfield,
    pub uploaded: usize,
    pub downloaded: usize,
    pub id: usize,
    pub throttle: Throttle,
    tracker: TrackerStatus,
    tracker_update: Option<Instant>,
    reg: Arc<amy::Registrar>,
    peers: UnsafeCell<HashMap<usize, Peer>>,
    leechers: HashSet<usize>,
    picker: Picker,
    pb: ProgressBar<io::Stdout>,
    paused: bool,
    unchoked: Vec<usize>,
    interested: HashSet<usize>,
    unchoke_timer: Instant,
}

impl Torrent {
    pub fn new(id: usize, info: Info, throttle: Throttle, reg: Arc<amy::Registrar>) -> Torrent {
        println!("Handling with torrent with {:?} pl, {:?} pieces, {:?} sf len", info.piece_len, info.pieces(), info.files.last().unwrap().length);
        // Create dummy files
        info.create_files().unwrap();
        let peers = UnsafeCell::new(HashMap::new());
        let pieces = Bitfield::new(info.pieces() as u64);
        let picker = Picker::new_rarest(&info);
        let pb = ProgressBar::new(info.pieces() as u64);
        let leechers = HashSet::new();
        let t = Torrent {
            id, info, peers, pieces, picker, pb,
            uploaded: 0, downloaded: 0, reg, leechers, throttle,
            paused: false, tracker: TrackerStatus::Updating,
            tracker_update: None, unchoked: Vec::new(),
            interested: HashSet::new(), unchoke_timer: Instant::now(),
        };
        TRACKER.tx.send(tracker::Request::started(&t)).unwrap();
        t
    }

    pub fn set_tracker_response(&mut self, resp: &TrackerRes) {
        match resp {
            &Ok(ref r) => {
                let mut time = Instant::now();
                time += Duration::from_secs(r.interval as u64);
                self.tracker = TrackerStatus::Ok { seeders: r.seeders, leechers: r.leechers, interval: r.interval };
                self.tracker_update = Some(time);
            }
            &Err(ref e) => {
                self.tracker = TrackerStatus::Error(e.clone());
            }
        }
    }

    pub fn update_tracker(&mut self) {
        if let Some(end) = self.tracker_update {
            let cur = Instant::now();
            if cur >= end {
                TRACKER.tx.send(tracker::Request::interval(&self)).unwrap();
            }
        }
    }

    pub fn block_available(&mut self, pid: usize, resp: disk::Response) -> io::Result<()> {
        let peer = self.peers().get_mut(&pid).unwrap();
        let ctx = resp.context;
        let p = Message::s_piece(ctx.idx, ctx.begin, ctx.length, resp.data);
        // This may not be 100% accurate, but close enough for now.
        self.uploaded += 1;
        peer.send_message(p)?;
        Ok(())
    }

    pub fn peer_readable(&mut self, pid: usize) -> io::Result<()> {
        let peer = self.peers().get_mut(&pid).unwrap();
        for msg in peer.readable()? {
            self.handle_msg(msg, pid)?;
        }
        Ok(())
    }

    pub fn handle_msg(&mut self, msg: Message, pid: usize) -> io::Result<()> {
        let peer = self.peers().get_mut(&pid).unwrap();
        match msg {
            Message::Handshake { .. } => {
                println!("Connection established with peer {:?}", peer.id);
            }
            Message::Bitfield(mut pf) => {
                pf.cap(self.pieces.len());
                peer.pieces = pf;
                if self.pieces.usable(&peer.pieces) {
                    self.picker.add_peer(&peer);
                    peer.interesting = true;
                    peer.send_message(Message::Interested)?;
                }
                if !peer.pieces.complete() {
                    self.leechers.insert(peer.id);
                }
            }
            Message::Have(idx) => {
                if idx >= self.pieces.len() as u32 {
                    return io_err("Invalid piece provided in HAVE");
                }
                peer.pieces.set_bit(idx as u64);
                if peer.pieces.complete() && self.leechers.contains(&peer.id) {
                    self.leechers.remove(&peer.id);
                }
                if !peer.interesting && self.pieces.usable(&peer.pieces) {
                    peer.interesting = true;
                    peer.send_message(Message::Interested)?;
                }
                self.picker.piece_available(idx);
            }
            Message::Unchoke => {
                peer.being_choked = false;
                Torrent::make_requests(&mut self.picker, peer, &self.info)?;
            }
            Message::Choke => {
                peer.being_choked = true;
            }
            Message::Piece { index, begin, data, length } => {
                if self.pieces.complete() || self.pieces.has_bit(index as u64) {
                    return Ok(());
                }

                peer.queued -= 1;
                Torrent::write_piece(&self.info, index, begin, length, data);
                let (piece_done, mut peers) = self.picker.completed(index, begin);
                if piece_done {
                    self.downloaded += 1;
                    self.pb.inc();
                    self.pieces.set_bit(index as u64);
                    if self.pieces.complete() {
                        TRACKER.tx.send(tracker::Request::completed(&self)).unwrap();
                        self.pb.finish_print("Downloaded!");
                    }
                    let m = Message::Have(index);
                    for pid in self.leechers.iter() {
                        let peer = self.peers().get_mut(pid).expect("Seeder IDs should be in peers");
                        if !peer.pieces.has_bit(index as u64) {
                            if peer.send_message(m.clone()).is_err() {
                                // TODO resolve the locality issue here,
                                // if we remove the torrent we can't remove it
                                // later.
                            }
                        }
                    }
                }
                if peers.len() > 1 {
                    peers.remove(&peer.id);
                    let m = Message::Cancel { index, begin, length };
                    for pid in peers {
                        if let Some(peer) = self.peers().get_mut(&pid) {
                            if let Err(_) = peer.send_message(m.clone()) {
                                self.remove_peer(pid);
                            }
                        }
                    }
                }
                if !peer.being_choked && !self.pieces.complete() && !self.paused {
                    Torrent::make_requests(&mut self.picker, peer, &self.info)?;
                }
            }
            Message::Request { index, begin, length } => {
                // TODO get this from some sort of allocator.
                if !peer.choked {
                    Torrent::request_read(peer.id, &self.info, index, begin, length, Box::new([0u8; 16384]));
                } else {
                    return io_err("Peer requested while choked!");
                }
            }
            Message::Cancel { .. } => {
                // TODO create some sort of filter so that when we finish reading a cancel'd piece
                // it never gets sent.
            }
            Message::Interested => {
                peer.interested = true;
                if self.unchoked.len() < 4 {
                    // Add to unchoked, and reset the uploaded/downloaded counter
                    // so that it may be fairly compared to
                    self.unchoked.push(peer.id);
                    peer.downloaded = 0;
                    peer.uploaded = 0;
                    peer.choked = false;
                    peer.send_message(Message::Unchoke)?;
                } else {
                    self.interested.insert(peer.id);
                }
            }
            Message::Uninterested => {
                peer.interested = false;
                self.interested.remove(&peer.id);
                if !peer.choked {
                    peer.choked = true;
                    peer.send_message(Message::Choke)?;
                }
            }
            _ => { }
        }
        Ok(())
    }

    /// Periodically called to update peers, choking the slowest one and
    /// optimistically unchoking a new peer
    pub fn update_unchoked(&mut self) {
        if self.unchoke_timer.elapsed() < Duration::from_secs(10) || self.unchoked.len() < 5 {
            return;
        }
        let (slowest, _) = if self.pieces.complete() {
            // Seed mode unchoking, seed to 4 fastest downloaders and 1 rotated random
            self.unchoked.iter().enumerate().fold((0, std::usize::MAX), |(slowest, min), (idx, id)| {
                let dl = self.peers()[id].uploaded;
                self.peers().get_mut(id).unwrap().uploaded = 0;
                if dl < min {
                    (idx, dl)
                } else {
                    (slowest, min)
                }
            })
        } else {
            // Leech mode unchoking, seed to 4 fastest uploaders and 1 rotated random
            self.unchoked.iter().enumerate().fold((0, std::usize::MAX), |(slowest, min), (idx, id)| {
                let dl = self.peers()[id].downloaded;
                self.peers().get_mut(id).unwrap().downloaded = 0;
                if dl < min {
                    (idx, dl)
                } else {
                    (slowest, min)
                }
            })
        };
        let slowest_id = self.unchoked.remove(slowest);
        let peer = self.peers().get_mut(&slowest_id).unwrap();
        peer.choked = true;
        // TODO: Handle locality here properly
        if peer.send_message(Message::Choke).is_err() {
        
        }
        self.interested.insert(slowest_id);

        // Unchoke one random interested peer
        let random_id = *random_sample(self.interested.iter()).unwrap();
        let peer = self.peers().get_mut(&random_id).unwrap();
        peer.choked = false;
        if peer.send_message(Message::Unchoke).is_err() {
        
        }
        self.interested.remove(&random_id);
        self.unchoked.push(random_id);
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

    #[inline]
    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(info: &Info, index: u32, begin: u32, len: u32, data: Box<[u8; 16384]>) {
        let locs = Torrent::calc_block_locs(info, index, begin, len);
        DISK.tx.send(disk::Request::write(data, locs)).unwrap();
    }

    #[inline]
    /// Issues a read request of the given torrent
    fn request_read(id: usize, info: &Info, index: u32, begin: u32, len: u32, data: Box<[u8; 16384]>) {
        let locs = Torrent::calc_block_locs(info, index, begin, len);
        let ctx = disk::Ctx::new(id, index, begin, len);
        DISK.tx.send(disk::Request::read(ctx, data, locs)).unwrap();
    }

    #[inline]
    fn make_requests(picker: &mut Picker, peer: &mut Peer, info: &Info) -> io::Result<()> {
        // keep 5 outstanding requests?
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

    pub fn peer_writable(&mut self, pid: usize) -> io::Result<()> {
        let peer = self.peers().get_mut(&pid).unwrap();
        peer.writable()?;
        Ok(())
    }

    pub fn rpc_info(&self) -> rpc::TorrentInfo {
        let status = if self.paused {
            rpc::Status::Paused
        } else if self.pieces.complete() {
            rpc::Status::Seeding
        } else {
            rpc::Status::Downloading
        };
        rpc::TorrentInfo {
            name: self.info.name.clone(),
            size: self.info.total_len,
            downloaded: self.downloaded as u64 * self.info.piece_len as u64,
            uploaded: self.uploaded as u64 * self.info.piece_len as u64,
            tracker: self.info.announce.clone(),
            tracker_status: self.tracker.clone(),
            status: status,
        }
    }

    pub fn file_size(&self) -> usize {
        let mut size = 0;
        for file in self.info.files.iter() {
            size += file.length;
        }
        size
    }

    pub fn add_peer(&mut self, mut peer: Peer) -> Option<usize> {
        let pid = self.reg.register(&peer.conn, amy::Event::Both).unwrap();
        peer.id = pid;
        if let Ok(()) = peer.set_torrent(&self) {
            self.picker.add_peer(&peer);
            self.peers().insert(peer.id, peer);
            Some(pid)
        } else {
            self.reg.deregister(&peer.conn).unwrap();
            None
        }
    }

    pub fn remove_peer(&mut self, id: usize) -> Peer {
        let peer = self.peers().remove(&id).unwrap();
        self.reg.deregister(&peer.conn).unwrap();
        self.leechers.remove(&id);
        self.picker.remove_peer(&peer);
        peer
    }

    pub fn pause(&mut self) {
        if !self.paused {
            TRACKER.tx.send(tracker::Request::stopped(&self)).unwrap();
        }
        self.paused = true;
    }

    pub fn resume(&mut self) {
        if self.paused {
            TRACKER.tx.send(tracker::Request::started(&self)).unwrap();
        }
        let mut failed = Vec::new();
        for (id, peer) in self.peers().iter_mut() {
            if Torrent::make_requests(&mut self.picker, peer, &self.info).is_err() {
                failed.push(id);
            }
        }
        for id in failed {
            self.remove_peer(*id);
        }
        self.paused = false;
    }

    pub fn change_picker(&mut self, mut picker: Picker) {
        for (_, peer) in self.peers().iter() {
            picker.add_peer(peer);
        }
        self.picker = picker;
    }

    // This obviously could be dangerous, but as long as we:
    // 1. keep the returned references within the scope of implemented methods
    // 2. don't invalidate iterators
    // it's more or less guaranteed to be safe.
    fn peers<'f>(&self) -> &'f mut HashMap<usize, Peer> {
        unsafe {
            self.peers.get().as_mut().unwrap()
        }
    }
}

impl fmt::Debug for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {{ info: {:?} }}", self.info)
    }
}


impl Drop for Torrent {
    fn drop(&mut self) {
        for (id, peer) in self.peers().drain() {
            self.reg.deregister(&peer.conn).unwrap();
            self.leechers.remove(&id);
        }
        if !self.paused {
            TRACKER.tx.send(tracker::Request::stopped(&self)).unwrap();
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub enum TrackerStatus {
    Updating,
    Ok { seeders: u32, leechers: u32, interval: u32 },
    Error(TrackerError),
}
