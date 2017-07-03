pub mod info;
pub mod peer;
pub mod bitfield;
mod picker;
mod choker;

pub use self::bitfield::Bitfield;
pub use self::info::Info;
pub use self::peer::{Peer, PeerConn};

use self::peer::Message;
use self::picker::Picker;
use std::fmt;
use {amy, bincode, rpc, disk, DISK, TRACKER};
use throttle::Throttle;
use tracker::{self, TrackerResponse};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use util::{io_err_val, torrent_name};
use std::cell::UnsafeCell;
use slog::Logger;

#[derive(Clone, Debug, Serialize)]
pub enum TrackerStatus {
    Updating,
    Ok { seeders: u32, leechers: u32, interval: u32 },
    Failure(String),
    Error,
}

#[derive(Serialize, Deserialize)]
struct TorrentData {
    info: Info,
    pieces: Bitfield,
    uploaded: usize,
    downloaded: usize,
    picker: Picker,
    status: Status,
}

pub struct Torrent {
    pieces: Bitfield,
    info: Arc<Info>,
    id: usize,
    downloaded: usize,
    uploaded: usize,
    throttle: Throttle,
    tracker: TrackerStatus,
    tracker_update: Option<Instant>,
    reg: Arc<amy::Registrar>,
    peers: UnsafeCell<HashMap<usize, Peer>>,
    leechers: HashSet<usize>,
    picker: Picker,
    status: Status,
    choker: choker::Choker,
    l: Logger,
    dirty: bool,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum Status {
    Pending,
    Paused,
    Leeching,
    Idle,
    Seeding,
    Validating,
    DiskError,
}

impl Status {
    pub fn stopped(&self) -> bool {
        match *self {
            Status::Paused | Status::DiskError => true,
            _ => false
        }
    }
}

impl Torrent {
    pub fn new(id: usize, info: Info, throttle: Throttle, reg: Arc<amy::Registrar>, l: Logger) -> Torrent {
        debug!(l, "Creating {:?}", info);
        // Create empty initial files
        info.create_files().unwrap();
        let peers = UnsafeCell::new(HashMap::new());
        let pieces = Bitfield::new(info.pieces() as u64);
        let picker = Picker::new_rarest(&info);
        let leechers = HashSet::new();
        let t = Torrent {
            id, info: Arc::new(info), peers, pieces, picker,
            uploaded: 0, downloaded: 0, reg, leechers, throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None, choker: choker::Choker::new(),
            l: l.clone(), dirty: false, status: Status::Pending,
        };
        debug!(l, "Sending start request");
        TRACKER.tx.send(tracker::Request::started(&t)).unwrap();
        t
    }

    pub fn deserialize(id: usize, data: &[u8], throttle: Throttle, reg: Arc<amy::Registrar>, l: Logger) -> Result<Torrent, bincode::Error> {
        let mut d: TorrentData = bincode::deserialize(data)?;
        debug!(l, "Torrent data deserialized!");
        d.picker.unset_waiting();
        let peers = UnsafeCell::new(HashMap::new());
        let leechers = HashSet::new();
        let t = Torrent {
            id, info: Arc::new(d.info), peers, pieces: d.pieces, picker: d.picker,
            uploaded: d.uploaded, downloaded: d.downloaded, reg, leechers, throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None, choker: choker::Choker::new(),
            l: l.clone(), dirty: false, status: d.status
        };
        if let Status::Validating = d.status {
            DISK.tx.send(disk::Request::validate(t.id, t.info.clone())).unwrap();
        }
        debug!(l, "Sending start request");
        TRACKER.tx.send(tracker::Request::started(&t)).unwrap();
        Ok(t)
    }

    pub fn serialize(&mut self) {
        let d = TorrentData {
            info: self.info.as_ref().clone(),
            pieces: self.pieces.clone(),
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            picker: self.picker.clone(),
            status: self.status,
        };
        let data = bincode::serialize(&d, bincode::Infinite).unwrap();
        debug!(self.l, "Sending serialization request!");
        DISK.tx.send(disk::Request::serialize(self.id, data, self.info.hash)).unwrap();
        self.dirty = false;
    }

    pub fn delete(&self) {
        debug!(self.l, "Sending file deletion request!");
        DISK.tx.send(disk::Request::delete(self.id, self.info.hash)).unwrap();
    }

    pub fn set_tracker_response(&mut self, resp: &tracker::Result<TrackerResponse>) {
        debug!(self.l, "Processing tracker response");
        match *resp {
            Ok(ref r) => {
                let mut time = Instant::now();
                time += Duration::from_secs(r.interval as u64);
                self.tracker = TrackerStatus::Ok { seeders: r.seeders, leechers: r.leechers, interval: r.interval };
                self.tracker_update = Some(time);
            }
            Err(tracker::Error(tracker::ErrorKind::TrackerError(ref s), _)) => {
                self.tracker = TrackerStatus::Failure(s.clone());
            }
            Err(e) => {
                warn!(self.l, "Failed to query tracker: {:?}", e.backtrace());
                self.tracker = TrackerStatus::Error;
            }
        }
    }

    pub fn update_tracker(&mut self) {
        if let Some(end) = self.tracker_update {
            debug!(self.l, "Updating tracker at inteval!");
            let cur = Instant::now();
            if cur >= end {
                TRACKER.tx.send(tracker::Request::interval(self)).unwrap();
            }
        }
    }

    pub fn reap_peers(&mut self) {
        debug!(self.l, "Reaping peers");
        let to_remove = self.peers().iter().filter_map(|(i, p)| p.error().map(|_| *i)).collect::<Vec<_>>();
        for pid in to_remove {
            self.remove_peer(pid);
        }
    }

    pub fn get_throttle(&self, id: usize) -> Throttle {
        self.throttle.new_sibling(id)
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn uploaded(&self) -> usize {
        self.uploaded
    }

    pub fn downloaded(&self) -> usize {
        self.downloaded
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn handle_disk_resp(&mut self, resp: disk::Response) {
        match resp {
            disk::Response::Read { context, data } => {
                trace!(self.l, "Received piece from disk, uploading!");
                if let Some(peer) = self.peers().get_mut(&context.pid) {
                    let p = Message::s_piece(context.idx, context.begin, context.length, data);
                    // This may not be 100% accurate, but close enough for now.
                    self.uploaded += 1;
                    self.dirty = true;
                    peer.send_message(p);
                }
            }
            disk::Response::ValidationComplete { invalid, .. } => {
                debug!(self.l, "Validation completed!");
                if invalid.is_empty() {
                    info!(self.l, "Torrent succesfully downloaded!");
                    self.status = Status::Idle;
                    TRACKER.tx.send(tracker::Request::completed(self)).unwrap();
                } else {
                    warn!(self.l, "Torrent has incorrect pieces {:?}, redownloading", invalid);
                    for piece in invalid {
                        self.picker.invalidate_piece(piece);
                    }
                    for (_, peer) in self.peers().iter_mut() {
                        self.make_requests(peer);
                    }
                }
            }
            disk::Response::Error { err, .. } => {
                warn!(self.l, "Disk error: {:?}", err);
                self.status = Status::DiskError;
            }
        }
    }

    pub fn peer_readable(&mut self, pid: usize) {
        let peer = self.peers().get_mut(&pid).unwrap();
        while let Some(msg) = peer.read() {
            self.handle_msg(msg, pid);
        }
    }

    pub fn handle_msg(&mut self, msg: Message, pid: usize) {
        let mut peer = self.peers().get_mut(&pid).unwrap();
        trace!(self.l, "Received {:?} from peer", msg);
        match msg {
            Message::Handshake { .. } => {
                debug!(self.l, "Connection established with peer {:?}", peer.id());
            }
            Message::Bitfield(_) => {
                if self.pieces.usable(peer.pieces()) {
                    self.picker.add_peer(peer);
                    peer.interested();
                }
                if !peer.pieces().complete() {
                    self.leechers.insert(peer.id());
                }
            }
            Message::Have(idx) => {
                if peer.pieces().complete() {
                    self.leechers.remove(&peer.id());
                }
                if self.pieces.usable(peer.pieces()) {
                    peer.interested();
                }
                self.picker.piece_available(idx);
            }
            Message::Unchoke => {
                debug!(self.l, "Unchoked by: {:?}!", peer);
                self.make_requests(peer);
            }
            Message::Piece { index, begin, data, length } => {
                // Ignore a piece we already have, this could happen from endgame
                if self.pieces.has_bit(index as u64) {
                    return;
                }


                // Even though we have the data, if we are stopped we shouldn't use the disk
                // regardless.
                if !self.status.stopped() {
                    self.status = Status::Leeching;
                } else {
                    return;
                }

                if self.info.block_len(index, begin) != length {
                    peer.set_error(io_err_val("Peer returned block of invalid len!"));
                    return;
                }

                // Internal data structures which are being serialized have changed, flag self as
                // dirty
                self.dirty = true;
                self.write_piece(index, begin, data);

                let (piece_done, mut peers) = self.picker.completed(index, begin);
                if piece_done {
                    self.downloaded += 1;
                    self.pieces.set_bit(index as u64);

                    // Begin validation if the torrent is done
                    if self.pieces.complete() {
                        DISK.tx.send(disk::Request::validate(self.id, self.info.clone())).unwrap();
                        self.status = Status::Validating;
                    }

                    // Tell all relevant peers we got the piece
                    let m = Message::Have(index);
                    for pid in self.leechers.iter() {
                        let peer = self.peers().get_mut(pid).expect("Seeder IDs should be in peers");
                        if !peer.pieces().has_bit(index as u64) {
                            peer.send_message(m.clone());
                        }
                    }
                }

                // If there are any peers we've asked duplicate pieces for(due to endgame),
                // cancel it, though we should still assume they'll probably send it anyways
                if peers.len() > 1 {
                    peers.remove(&peer.id());
                    let m = Message::Cancel { index, begin, length };
                    for pid in peers {
                        if let Some(peer) = self.peers().get_mut(&pid) {
                            peer.send_message(m.clone());
                        }
                    }
                }

                if !self.pieces.complete() {
                    self.make_requests(peer);
                }
            }
            Message::Request { index, begin, length } => {
                if !self.status.stopped() {
                    self.status = Status::Seeding;
                    // TODO get this from some sort of allocator.
                    if length != self.info.block_len(index, begin) {
                        peer.set_error(io_err_val("Peer requested block of invalid len!"));
                    } else {
                        self.request_read(peer.id(), index, begin, Box::new([0u8; 16384]));
                    }
                } else {
                // TODO: add this to a queue to fulfill later
                }
            }
            Message::Interested => {
                self.choker.add_peer(&mut peer);
            }
            Message::Uninterested => {
                let peers = self.peers();
                self.choker.remove_peer(&mut peer, peers);
            }
            _ => { }
        }
    }

    /// Periodically called to update peers, choking the slowest one and
    /// optimistically unchoking a new peer
    pub fn update_unchoked(&mut self) {
        let peers = self.peers();
        if self.complete() {
            self.choker.update_download(peers)
        } else {
            self.choker.update_upload(peers)
        };
    }

    fn complete(&self) -> bool {
        self.pieces.complete()
    }

    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(&self, index: u32, begin: u32, data: Box<[u8; 16384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        DISK.tx.send(disk::Request::write(self.id, data, locs)).unwrap();
    }

    /// Issues a read request of the given torrent
    fn request_read(&self, id: usize, index: u32, begin: u32, data: Box<[u8; 16384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        let len = self.info.block_len(index, begin);
        let ctx = disk::Ctx::new(id, self.id, index, begin, len);
        DISK.tx.send(disk::Request::read(ctx, data, locs)).unwrap();
    }

    fn make_requests(&mut self, peer: &mut Peer) {
        if self.status.stopped() {
            return;
        }
        while peer.can_queue_req() {
            if let Some((idx, offset)) = self.picker.pick(peer) {
                peer.request_piece(idx, offset, self.info.block_len(idx, offset));
            } else {
                break;
            }
        }
    }

    pub fn peer_writable(&mut self, pid: usize) {
        self.peers().get_mut(&pid).unwrap().writable();
    }

    pub fn rpc_info(&self) -> rpc::TorrentInfo {
        rpc::TorrentInfo {
            name: self.info.name.clone(),
            size: self.info.total_len,
            downloaded: self.downloaded as u64 * self.info.piece_len as u64,
            uploaded: self.uploaded as u64 * self.info.piece_len as u64,
            tracker: self.info.announce.clone(),
            tracker_status: self.tracker.clone(),
            status: self.status,
        }
    }

    pub fn file_size(&self) -> usize {
        let mut size = 0;
        for file in self.info.files.iter() {
            size += file.length;
        }
        size
    }

    pub fn add_peer(&mut self, conn: PeerConn) -> Option<usize> {
        let pid = self.reg.register(conn.sock(), amy::Event::Both).unwrap();
        debug!(self.l, "Adding peer {:?}!", pid);
        let p = Peer::new(pid, conn, self);
        if p.error().is_none() {
            self.picker.add_peer(&p);
            self.peers().insert(pid, p);
            // We want to make sure any queued up messages are drained once we register
            self.peer_readable(pid);
            Some(pid)
        } else {
            self.reg.deregister(p.conn().sock()).unwrap();
            return None;
        }
    }

    pub fn remove_peer(&mut self, id: usize) -> Peer {
        debug!(self.l, "Removing peer {:?}!", id);
        let mut peer = self.peers().remove(&id).unwrap();
        let peers = self.peers();
        self.choker.remove_peer(&mut peer, peers);
        self.reg.deregister(peer.conn().sock()).unwrap();
        self.leechers.remove(&id);
        self.picker.remove_peer(&peer);
        peer
    }

    pub fn pause(&mut self) {
        debug!(self.l, "Pausing torrent!");
        match self.status {
            Status::Paused => { }
            _ => {
                debug!(self.l, "Sending stopped request to trk");
                TRACKER.tx.send(tracker::Request::stopped(self)).unwrap();
            }
        }
        self.status = Status::Paused;
    }

    pub fn resume(&mut self) {
        debug!(self.l, "Resuming torrent!");
        match self.status {
            Status::Paused => {
                debug!(self.l, "Sending started request to trk");
                TRACKER.tx.send(tracker::Request::started(self)).unwrap();
                for (_, peer) in self.peers().iter_mut() {
                    self.make_requests(peer);
                }
            }
            Status::DiskError => {
                if self.pieces.complete() {
                    DISK.tx.send(disk::Request::validate(self.id, self.info.clone())).unwrap();
                    self.status = Status::Validating;
                } else {
                    for (_, peer) in self.peers().iter_mut() {
                        self.make_requests(peer);
                    }
                    self.status = Status::Idle;
                }
            }
            _ => { }
        }
        if self.pieces.complete() {
            self.status = Status::Idle;
        } else {
            self.status = Status::Pending;
        }
    }

    pub fn change_picker(&mut self, mut picker: Picker) {
        debug!(self.l, "Swapping pickers!");
        for (_, peer) in self.peers().iter() {
            picker.add_peer(peer);
        }
        self.picker.change_picker(picker);
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

impl fmt::Display for Torrent {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {}", torrent_name(&self.info.hash))
    }
}

impl Drop for Torrent {
    fn drop(&mut self) {
        debug!(self.l, "Deregistering peers");
        for (id, peer) in self.peers().drain() {
            trace!(self.l, "Deregistering peer {:?}", peer);
            self.reg.deregister(peer.conn().sock()).unwrap();
            self.leechers.remove(&id);
        }
        match self.status {
            Status::Paused => { }
            _ => {
                TRACKER.tx.send(tracker::Request::stopped(self)).unwrap();
            }
        }
    }
}
