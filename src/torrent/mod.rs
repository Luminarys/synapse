pub mod info;
pub mod peer;
pub mod bitfield;
mod picker;
mod choker;

pub use self::bitfield::Bitfield;
pub use self::info::Info;
pub use self::peer::{Peer, PeerConn};

pub use self::peer::Message;
use self::picker::Picker;
use std::fmt;
use control::cio;
use {bincode, rpc, disk, RAREST_PKR};
use throttle::Throttle;
use tracker::{self, TrackerResponse};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use util::torrent_name;
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

pub struct Torrent<T: cio::CIO> {
    id: usize,
    pieces: Bitfield,
    info: Arc<Info>,
    cio: T,
    downloaded: usize,
    uploaded: usize,
    throttle: Throttle,
    tracker: TrackerStatus,
    tracker_update: Option<Instant>,
    peers: HashMap<usize, Peer<T>>,
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

impl<T: cio::CIO> Torrent<T> {
    pub fn new(id: usize, info: Info, throttle: Throttle, cio: T, l: Logger) -> Torrent<T> {
        debug!(l, "Creating {:?}", info);
        // Create empty initial files
        let created = info.create_files().ok();
        let peers = HashMap::new();
        let pieces = Bitfield::new(info.pieces() as u64);
        let picker = if RAREST_PKR {
            Picker::new_rarest(&info)
        } else {
            Picker::new_sequential(&info)
        };
        let leechers = HashSet::new();
        let status = if created.is_some() {
            Status::Pending
        } else {
            Status::DiskError
        };
        let mut t = Torrent {
            id, info: Arc::new(info), peers, pieces, picker,
            uploaded: 0, downloaded: 0, cio, leechers, throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None, choker: choker::Choker::new(),
            l: l.clone(), dirty: false, status,
        };
        t.start();
        t
    }

    pub fn deserialize(id: usize, data: &[u8], throttle: Throttle, cio: T, l: Logger) -> Result<Torrent<T>, bincode::Error> {
        let mut d: TorrentData = bincode::deserialize(data)?;
        debug!(l, "Torrent data deserialized!");
        d.picker.unset_waiting();
        let peers = HashMap::new();
        let leechers = HashSet::new();
        let mut t = Torrent {
            id, info: Arc::new(d.info), peers, pieces: d.pieces, picker: d.picker,
            uploaded: d.uploaded, downloaded: d.downloaded, cio, leechers, throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None, choker: choker::Choker::new(),
            l: l.clone(), dirty: false, status: d.status
        };
        if let Status::Validating = d.status {
            t.cio.msg_disk(disk::Request::validate(t.id, t.info.clone()));
        }
        t.start();
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
        let data = bincode::serialize(&d, bincode::Infinite).expect("Serialization failed!");
        debug!(self.l, "Sending serialization request!");
        self.cio.msg_disk(disk::Request::serialize(self.id, data, self.info.hash));
        self.dirty = false;
    }

    pub fn delete(&mut self) {
        debug!(self.l, "Sending file deletion request!");
        self.cio.msg_disk(disk::Request::delete(self.id, self.info.hash));
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
            Err(ref e) => {
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
                let req = tracker::Request::interval(self);
                self.cio.msg_trk(req);
            }
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
                if let Some(peer) = self.peers.get_mut(&context.pid) {
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
                    let req = tracker::Request::completed(self);
                    self.cio.msg_trk(req);
                } else {
                    warn!(self.l, "Torrent has incorrect pieces {:?}, redownloading", invalid);
                    for piece in invalid {
                        self.picker.invalidate_piece(piece);
                    }
                    self.request_all();
                }
            }
            disk::Response::Error { err, .. } => {
                warn!(self.l, "Disk error: {:?}", err);
                self.status = Status::DiskError;
            }
        }
    }

    pub fn peer_ev(&mut self, pid: cio::PID, evt: cio::Result<Message>) -> Result<(), ()> {
        let mut peer = self.peers.remove(&pid).ok_or(())?;
        if let Ok(mut msg) = evt {
            if peer.handle_msg(&mut msg).is_ok() && self.handle_msg(msg, &mut peer).is_ok() {
                self.peers.insert(pid, peer);
                return Ok(());
            }
        }
        self.cleanup_peer(&mut peer);
        Err(())
    }

    pub fn handle_msg(&mut self, msg: Message, peer: &mut Peer<T>) -> Result<(), ()> {
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
                    return Ok(());
                }

                // Even though we have the data, if we are stopped we shouldn't use the disk
                // regardless.
                if !self.status.stopped() {
                    self.status = Status::Leeching;
                } else {
                    return Ok(());
                }

                if self.info.block_len(index, begin) != length {
                    return Err(());
                }

                // Internal data structures which are being serialized have changed, flag self as
                // dirty
                self.dirty = true;
                self.write_piece(index, begin, data);

                let (piece_done, mut peers) = self.picker.completed(index, begin);
                if piece_done {
                    self.downloaded += 1;
                    self.pieces.set_bit(index as u64);

                    // Begin validation, and save state if the torrent is done
                    if self.pieces.complete() {
                        self.serialize();
                        self.cio.msg_disk(disk::Request::validate(self.id, self.info.clone()));
                        self.status = Status::Validating;
                    }

                    // Tell all relevant peers we got the piece
                    let m = Message::Have(index);
                    for pid in self.leechers.iter() {
                        if let Some(peer) = self.peers.get_mut(pid) {
                            if !peer.pieces().has_bit(index as u64) {
                                peer.send_message(m.clone());
                            }
                        } else {
                            warn!(self.l, "PID {} in leechers not found in peers.", pid);
                        }
                    }

                    // Mark uninteresting peers
                    for (_, peer) in self.peers.iter_mut() {
                        if !self.pieces.usable(peer.pieces()) {
                            peer.uninterested();
                        }
                    }
                }

                // If there are any peers we've asked duplicate pieces for(due to endgame),
                // cancel it, though we should still assume they'll probably send it anyways
                if peers.len() > 1 {
                    peers.remove(&peer.id());
                    let m = Message::Cancel { index, begin, length };
                    for pid in peers {
                        if let Some(peer) = self.peers.get_mut(&pid) {
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
                        return Err(());
                    } else {
                        self.request_read(peer.id(), index, begin, Box::new([0u8; 16384]));
                    }
                } else {
                // TODO: add this to a queue to fulfill later
                }
            }
            Message::Interested => {
                self.choker.add_peer(peer);
            }
            Message::Uninterested => {
                self.choker.remove_peer(peer, &mut self.peers);
            }
            Message::KeepAlive
            | Message::Choke
            | Message::Cancel { .. }
            | Message::Port(_) => { }

            Message::SharedPiece { .. } => { unreachable!() }
        }
        Ok(())
    }

    /// Periodically called to update peers, choking the slowest one and
    /// optimistically unchoking a new peer
    pub fn update_unchoked(&mut self) {
        if self.complete() {
            self.choker.update_download(&mut self.peers)
        } else {
            self.choker.update_upload(&mut self.peers)
        };
    }

    fn start(&mut self) {
        debug!(self.l, "Sending start request");
        let req = tracker::Request::started(self);
        self.cio.msg_trk(req);
        // TODO: Consider repeatedly sending out these during annoucne intervals
        if !self.info.private {
            let mut req = tracker::Request::DHTAnnounce(self.info.hash);
            self.cio.msg_trk(req);
            req = tracker::Request::GetPeers(tracker::GetPeers { id: self.id, hash: self.info.hash });
            self.cio.msg_trk(req);
        }
    }

    fn complete(&self) -> bool {
        self.pieces.complete()
    }

    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(&mut self, index: u32, begin: u32, data: Box<[u8; 16384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        self.cio.msg_disk(disk::Request::write(self.id, data, locs));
    }

    /// Issues a read request of the given torrent
    fn request_read(&mut self, id: usize, index: u32, begin: u32, data: Box<[u8; 16384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        let len = self.info.block_len(index, begin);
        let ctx = disk::Ctx::new(id, self.id, index, begin, len);
        self.cio.msg_disk(disk::Request::read(ctx, data, locs));
    }

    fn make_requests_pid(&mut self, pid: usize) {
        let peer = self.peers.get_mut(&pid).expect("Expected peer id not present");
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

    fn make_requests(&mut self, peer: &mut Peer<T>) {
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

    pub fn rpc_info(&self) -> rpc::resource::SResourceUpdate {
        unimplemented!();
    }

    pub fn add_peer(&mut self, conn: PeerConn) -> Option<usize> {
        if let Ok(p) = Peer::new(conn, self) {
            let pid = p.id();
            debug!(self.l, "Adding peer {:?}!", pid);
            self.picker.add_peer(&p);
            self.peers.insert(pid, p);
            Some(pid)
        } else {
            None
        }
    }

    fn cleanup_peer(&mut self, peer: &mut Peer<T>) {
        debug!(self.l, "Removing peer {:?}!", peer);
        self.choker.remove_peer(peer, &mut self.peers);
        self.leechers.remove(&peer.id());
        self.picker.remove_peer(&peer);
    }

    pub fn pause(&mut self) {
        debug!(self.l, "Pausing torrent!");
        match self.status {
            Status::Paused => { }
            _ => {
                debug!(self.l, "Sending stopped request to trk");
                let req = tracker::Request::stopped(self);
                self.cio.msg_trk(req);
            }
        }
        self.status = Status::Paused;
    }

    pub fn resume(&mut self) {
        debug!(self.l, "Resuming torrent!");
        match self.status {
            Status::Paused => {
                debug!(self.l, "Sending started request to trk");
                let req = tracker::Request::started(self);
                self.cio.msg_trk(req);
                self.request_all();
            }
            Status::DiskError => {
                if self.pieces.complete() {
                    self.cio.msg_disk(disk::Request::validate(self.id, self.info.clone()));
                    self.status = Status::Validating;
                } else {
                    self.request_all();
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

    fn request_all(&mut self) {
        for pid in self.pids() {
            self.make_requests_pid(pid);
        }
    }

    fn pids(&self) -> Vec<usize> {
        self.peers.keys().cloned().collect()
    }

    // TODO: use this over RPC
    #[allow(dead_code)]
    pub fn change_picker(&mut self, mut picker: Picker) {
        debug!(self.l, "Swapping pickers!");
        for (_, peer) in self.peers.iter() {
            picker.add_peer(peer);
        }
        self.picker.change_picker(picker);
    }
}

impl<T: cio::CIO> fmt::Debug for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {{ info: {:?} }}", self.info)
    }
}

impl<T: cio::CIO> fmt::Display for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {}", torrent_name(&self.info.hash))
    }
}

impl<T: cio::CIO> Drop for Torrent<T> {
    fn drop(&mut self) {
        debug!(self.l, "Removing peers");
        for (id, peer) in self.peers.drain() {
            trace!(self.l, "Removing peer {:?}", peer);
            self.cio.remove_peer(id);
            self.leechers.remove(&id);
        }
        match self.status {
            Status::Paused => { }
            _ => {
                let req = tracker::Request::stopped(self);
                self.cio.msg_trk(req);
            }
        }
    }
}
