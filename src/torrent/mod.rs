pub mod info;
pub mod peer;
pub mod bitfield;
mod picker;
mod choker;

use std::fmt;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use slog::Logger;

pub use self::bitfield::Bitfield;
pub use self::info::Info;
pub use self::peer::{Peer, PeerConn};
pub use self::peer::Message;

use self::picker::Picker;
use {bincode, rpc, disk, util, RAREST_PKR, CONFIG};
use control::cio;
use rpc::resource::{self, Resource, SResourceUpdate};
use throttle::Throttle;
use tracker::{self, TrackerResponse};

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TrackerStatus {
    Updating,
    Ok {
        seeders: u32,
        leechers: u32,
        interval: u32,
    },
    Failure(String),
    Error,
}

#[derive(Serialize, Deserialize)]
struct TorrentData {
    info: Info,
    pieces: Bitfield,
    uploaded: u64,
    downloaded: u64,
    status: Status,
    path: Option<String>,
}

pub struct Torrent<T: cio::CIO> {
    id: usize,
    pieces: Bitfield,
    info: Arc<Info>,
    cio: T,
    uploaded: u64,
    downloaded: u64,
    last_ul: u64,
    last_dl: u64,
    priority: u8,
    last_clear: DateTime<Utc>,
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
    path: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    pub fn leeching(&self) -> bool {
        match *self {
            Status::Leeching => true,
            _ => false,
        }
    }

    pub fn stopped(&self) -> bool {
        match *self {
            Status::Paused | Status::DiskError => true,
            _ => false,
        }
    }
}

impl<T: cio::CIO> Torrent<T> {
    pub fn new(
        id: usize,
        path: Option<String>,
        info: Info,
        throttle: Throttle,
        cio: T,
        l: Logger,
    ) -> Torrent<T> {
        debug!(l, "Creating {:?}", info);
        let peers = HashMap::new();
        let pieces = Bitfield::new(info.pieces() as u64);
        let picker = if RAREST_PKR {
            Picker::new_rarest(&info, &pieces)
        } else {
            Picker::new_sequential(&info, &pieces)
        };
        let leechers = HashSet::new();
        let status = Status::Pending;
        let mut t = Torrent {
            id,
            info: Arc::new(info),
            path,
            peers,
            pieces,
            picker,
            priority: 3,
            uploaded: 0,
            downloaded: 0,
            last_ul: 0,
            last_dl: 0,
            last_clear: Utc::now(),
            cio,
            leechers,
            throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None,
            choker: choker::Choker::new(),
            l: l.clone(),
            dirty: true,
            status,
        };
        t.start();
        t.validate();

        t
    }

    pub fn deserialize(
        id: usize,
        data: &[u8],
        throttle: Throttle,
        cio: T,
        l: Logger,
    ) -> Result<Torrent<T>, bincode::Error> {
        let d: TorrentData = bincode::deserialize(data)?;
        debug!(l, "Torrent data deserialized!");
        let peers = HashMap::new();
        let leechers = HashSet::new();
        let picker = picker::Picker::new_rarest(&d.info, &d.pieces);
        let mut t = Torrent {
            id,
            info: Arc::new(d.info),
            peers,
            pieces: d.pieces,
            picker,
            uploaded: d.uploaded,
            downloaded: d.downloaded,
            last_ul: 0,
            last_dl: 0,
            priority: 3,
            last_clear: Utc::now(),
            cio,
            leechers,
            throttle,
            tracker: TrackerStatus::Updating,
            tracker_update: None,
            choker: choker::Choker::new(),
            l: l.clone(),
            dirty: false,
            status: d.status,
            path: d.path,
        };
        match t.status {
            Status::DiskError | Status::Seeding | Status::Leeching => {
                if t.pieces.complete() {
                    t.status = Status::Idle;
                } else {
                    t.status = Status::Pending;
                }
            }
            Status::Validating => {
                t.validate();
            }
            _ => {}
        };
        t.start();
        t.announce_start();
        Ok(t)
    }

    pub fn serialize(&mut self) {
        let d = TorrentData {
            info: self.info.as_ref().clone(),
            pieces: self.pieces.clone(),
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            status: self.status,
            path: self.path.clone(),
        };
        let data = bincode::serialize(&d, bincode::Infinite).expect("Serialization failed!");
        debug!(self.l, "Sending serialization request!");
        self.cio.msg_disk(disk::Request::serialize(
            self.id,
            data,
            self.info.hash,
        ));
        self.dirty = false;
    }

    pub fn rpc_id(&self) -> String {
        util::hash_to_id(&self.info.hash[..])
    }

    pub fn delete(&mut self) {
        debug!(self.l, "Sending file deletion request!");
        self.cio.msg_disk(
            disk::Request::delete(self.id, self.info.hash),
        );
    }

    pub fn set_tracker_response(&mut self, resp: &tracker::Result<TrackerResponse>) {
        debug!(self.l, "Processing tracker response");
        match *resp {
            Ok(ref r) => {
                let mut time = Instant::now();
                time += Duration::from_secs(r.interval as u64);
                self.tracker = TrackerStatus::Ok {
                    seeders: r.seeders,
                    leechers: r.leechers,
                    interval: r.interval,
                };
                self.tracker_update = Some(time);
            }
            Err(tracker::Error(tracker::ErrorKind::TrackerError(ref s), _)) => {
                self.tracker = TrackerStatus::Failure(s.clone());
            }
            Err(ref e) => {
                warn!(self.l, "Failed to query tracker: {:?}", e);
                self.tracker = TrackerStatus::Error;
            }
        }
        self.update_rpc_tracker();
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

    pub fn remove_peer(&mut self, rpc_id: &str) {
        let ih = &self.info.hash;
        let cio = &mut self.cio;
        self.peers
            .iter()
            .find(|&(id, _)| util::peer_rpc_id(ih, *id as u64) == rpc_id)
            .map(|(id, _)| cio.remove_peer(*id));
    }

    // TODO: Implement once mutlitracker support is in
    pub fn remove_tracker(&mut self, rpc_id: &str) {}

    pub fn get_throttle(&self, id: usize) -> Throttle {
        self.throttle.new_sibling(id)
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn dirty(&self) -> bool {
        self.dirty
    }

    pub fn uploaded(&self) -> u64 {
        self.uploaded
    }

    pub fn downloaded(&self) -> u64 {
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
                    self.uploaded += context.length as u64;
                    self.last_ul += context.length as u64;
                    self.dirty = true;
                    peer.send_message(p);
                }
            }
            disk::Response::ValidationComplete { invalid, .. } => {
                debug!(self.l, "Validation completed!");
                if invalid.is_empty() {
                    if !self.pieces.complete() {
                        for i in 0..self.pieces.len() {
                            self.pieces.set_bit(i);
                        }
                    }
                    info!(self.l, "Torrent succesfully downloaded!");
                    // TOOD: Consider if we should store this result
                    if !self.status.stopped() {
                        self.set_status(Status::Idle);
                    }
                    let req = tracker::Request::completed(self);
                    self.cio.msg_trk(req);
                    // Remove all seeding peers.
                    let leechers = &self.leechers;
                    let seeders = self.peers
                        .iter()
                        .filter(|&(id, _)| !leechers.contains(id))
                        .map(|(id, _)| *id);
                    for seeder in seeders {
                        self.cio.remove_peer(seeder);
                    }
                } else {
                    // If this is an initialization hash, start the torrent
                    // immediatly.
                    if !self.pieces.complete() {
                        // If there was some partial completion,
                        // set the pieces appropriately, then reset the
                        // picker to use the new bitfield
                        if invalid.len() as u64 != self.pieces.len() {
                            for i in 0..self.pieces.len() {
                                self.pieces.set_bit(i);
                            }
                            for piece in invalid {
                                self.pieces.unset_bit(piece as u64);
                            }
                            self.picker.refresh_picker(&self.pieces);
                        }
                        self.announce_start();
                    } else {
                        for piece in invalid {
                            self.picker.invalidate_piece(piece);
                            self.pieces.unset_bit(piece as u64);
                        }
                        self.request_all();
                    }
                    self.set_status(Status::Pending);
                }
                // update the RPC stats once done
                self.update_rpc_transfer();
            }
            disk::Response::Error { err, .. } => {
                warn!(self.l, "Disk error: {:?}", err);
                self.set_status(Status::DiskError);
            }
        }
    }

    pub fn peer_ev(&mut self, pid: cio::PID, evt: cio::Result<Message>) -> Result<(), ()> {
        // TODO: Consider Boxing peers so it's just pointer insert/removal
        let mut peer = self.peers.remove(&pid).ok_or(())?;
        if let Ok(mut msg) = evt {
            if peer.handle_msg(&mut msg).is_ok() && self.handle_msg(msg, &mut peer).is_ok() {
                self.peers.insert(pid, peer);
                return Ok(());
            } else {
                // In order to ensure there's only one source of truth,
                // we instruct the CIO to remove the peer and let the event bubble up.
                // We then will receieve it later and call cleanup_peer as needed.
                // This ensures that events flow from control -> torrent and get
                // properly processed
                self.cio.remove_peer(self.id);
            }
        } else {
            self.cleanup_peer(&mut peer);
        }
        Err(())
    }

    pub fn handle_msg(&mut self, msg: Message, peer: &mut Peer<T>) -> Result<(), ()> {
        trace!(self.l, "Received {:?} from peer", msg);
        match msg {
            Message::Bitfield(_) => {
                if self.pieces.usable(peer.pieces()) {
                    peer.interested();
                }
                self.picker.add_peer(peer);
                if !peer.pieces().complete() {
                    self.leechers.insert(peer.id());
                } else if self.complete() {
                    // Don't waste a connection on a peer if they're also a seeder
                    return Err(());
                }
            }
            Message::Have(idx) => {
                self.picker.piece_available(idx);
                if peer.pieces().complete() {
                    self.leechers.remove(&peer.id());
                    // If they're now a seeder and we're also seeding, drop the conn
                    if self.complete() {
                        return Err(());
                    }
                }
                if self.pieces.usable(peer.pieces()) {
                    peer.interested();
                }
            }
            Message::Unchoke => {
                self.make_requests(peer);
            }
            Message::Piece {
                index,
                begin,
                data,
                length,
            } => {
                // Ignore a piece we already have, this could happen from endgame
                if self.pieces.has_bit(index as u64) {
                    return Ok(());
                }

                // Even though we have the data, if we are stopped we shouldn't use the disk
                // regardless.
                if !self.status.stopped() {
                    self.set_status(Status::Leeching);
                } else {
                    return Ok(());
                }

                if self.info.block_len(index, begin) != length {
                    return Err(());
                }

                let pr = self.picker.completed(picker::Block::new(index, begin));
                let (piece_done, peers) = if let Ok(r) = pr {
                    r
                } else {
                    return Ok(());
                };

                // Internal data structures which are being serialized have changed, flag self as
                // dirty
                self.dirty = true;
                self.write_piece(index, begin, data);

                self.downloaded += length as u64;
                self.last_dl += length as u64;
                if piece_done {
                    self.pieces.set_bit(index as u64);
                    self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                        resource::SResourceUpdate::PieceDownloaded {
                            id: util::piece_rpc_id(&self.info.hash, index as u64),
                            downloaded: true,
                        },
                    ]));

                    // Begin validation, and save state if the torrent is done
                    if self.pieces.complete() {
                        debug!(self.l, "Beginning validation");
                        self.serialize();
                        self.validate();
                    }

                    // Tell all relevant peers we got the piece
                    let m = Message::Have(index);
                    for pid in &self.leechers {
                        if let Some(peer) = self.peers.get_mut(pid) {
                            if !peer.pieces().has_bit(index as u64) {
                                peer.send_message(m.clone());
                            }
                        } else {
                            // This situation can occur when a torrent itself is a leecher
                            // and the piece download causes a "self notification", while it
                            // has been removed. Ignore for now.
                        }
                    }

                    // Mark uninteresting peers
                    for peer in self.peers.values_mut() {
                        if !self.pieces.usable(peer.pieces()) {
                            peer.uninterested();
                        }
                    }
                }

                // If there are any peers we've asked duplicate pieces for,
                // cancel them, though we should still assume they'll probably send it anyways
                let m = Message::Cancel {
                    index,
                    begin,
                    length,
                };

                for pid in peers.into_iter().filter(|p| *p != peer.id()) {
                    if let Some(peer) = self.peers.get_mut(&pid) {
                        peer.send_message(m.clone());
                    }
                }

                if !self.pieces.complete() {
                    self.make_requests(peer);
                }
            }
            Message::Request {
                index,
                begin,
                length,
            } => {
                if !self.status.stopped() && !self.status.leeching() {
                    self.set_status(Status::Seeding);
                    // TODO get this from some sort of allocator.
                    if length != self.info.block_len(index, begin) {
                        return Err(());
                    } else {
                        self.request_read(peer.id(), index, begin, Box::new([0u8; 16_384]));
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

            // These messages are all handled at the peer level, not the torrent level,
            // so just ignore here
            Message::KeepAlive |
            Message::Choke |
            Message::Cancel { .. } |
            Message::Handshake { .. } |
            Message::Port(_) => {}

            Message::SharedPiece { .. } => unreachable!(),
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

    pub fn rpc_update(&mut self, u: rpc::proto::resource::CResourceUpdate) {
        if let Some(status) = u.status {
            match (status, self.status) {
                (resource::Status::Paused, Status::Paused) => {
                    self.resume();
                }
                (resource::Status::Paused, _) => {
                    self.pause();
                }
                (resource::Status::Hashing, _) => {
                    self.validate();
                }
                // The rpc module should handle invalid status requests.
                _ => {}
            }
        }

        if u.throttle_up.is_some() || u.throttle_down.is_some() {
            let tu = u.throttle_up.unwrap_or(self.throttle.ul_rate() as u32);
            let td = u.throttle_down.unwrap_or(self.throttle.dl_rate() as u32);
            self.set_throttle(tu, td);
        }

        if let Some(p) = u.path {
            self.set_path(p);
        }

        if let Some(p) = u.priority {
            self.set_priority(p);
        }

        if let Some(s) = u.sequential {
            self.change_picker(s);
        }
    }

    pub fn rpc_update_file(&mut self, id: String, priority: u8) {
        self.set_file_priority(id, priority);
    }

    fn start(&mut self) {
        debug!(self.l, "Starting torrent");
        // Update RPC of the torrent, tracker, files, and peers
        let resources = self.rpc_info();
        self.cio.msg_rpc(rpc::CtlMessage::Extant(resources));
        self.update_rpc_transfer();
        self.serialize();
    }

    fn announce_start(&mut self) {
        let req = tracker::Request::started(self);
        self.cio.msg_trk(req);
        // TODO: Consider repeatedly sending out these during annoucne intervals
        if !self.info.private {
            let mut req = tracker::Request::DHTAnnounce(self.info.hash);
            self.cio.msg_trk(req);
            req = tracker::Request::GetPeers(tracker::GetPeers {
                id: self.id,
                hash: self.info.hash,
            });
            self.cio.msg_trk(req);
        }
    }

    pub fn complete(&self) -> bool {
        match self.status {
            Status::Leeching | Status::Validating | Status::Pending => false,
            Status::Idle | Status::Seeding | Status::Paused => true,
            Status::DiskError => self.pieces.complete(),
        }
    }

    fn set_throttle(&mut self, ul: u32, dl: u32) {
        self.throttle.set_ul_rate(ul as usize);
        self.throttle.set_dl_rate(dl as usize);
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::Throttle {
                id,
                throttle_up: ul,
                throttle_down: dl,
            },
        ]));
    }

    fn set_path(&mut self, path: String) {
        // TODO: IMplement
    }

    fn set_priority(&mut self, priority: u8) {
        // TODO: Implement priority somewhere(throttle or ctrl)
        self.priority = priority;
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::TorrentPriority {
                id,
                priority,
            },
        ]));
    }

    fn set_file_priority(&mut self, id: String, priority: u8) {
        // TODO: Implement file priority in picker
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::FilePriority { id, priority },
        ]));
    }

    fn rpc_info(&self) -> Vec<resource::Resource> {
        let mut r = Vec::new();
        r.push(Resource::Torrent(resource::Torrent {
            id: self.rpc_id(),
            name: self.info.name.clone(),
            // TODO: Properly add this
            path: self.path.as_ref().unwrap_or(&CONFIG.disk.directory).clone(),
            created: Utc::now(),
            modified: Utc::now(),
            status: self.status.into(),
            error: self.error(),
            priority: 3,
            progress: self.progress(),
            availability: self.availability(),
            sequential: self.sequential(),
            rate_up: 0,
            rate_down: 0,
            // TODO: COnsider the overflow potential here
            throttle_up: self.throttle.ul_rate() as u32,
            throttle_down: self.throttle.dl_rate() as u32,
            transferred_up: self.uploaded,
            transferred_down: self.downloaded,
            peers: 0,
            // TODO: Alter when mutlitracker support hits
            trackers: 1,
            pieces: self.info.pieces() as u64,
            piece_size: self.info.piece_len,
            files: self.info.files.len() as u32,
        }));

        for i in 0..self.info.pieces() {
            let id = util::piece_rpc_id(&self.info.hash, i as u64);
            if self.pieces.has_bit(i as u64) {
                r.push(Resource::Piece(resource::Piece {
                    id,
                    torrent_id: self.rpc_id(),
                    available: true,
                    downloaded: true,
                }))
            } else {
                r.push(Resource::Piece(resource::Piece {
                    id,
                    torrent_id: self.rpc_id(),
                    available: true,
                    downloaded: false,
                }))
            }
        }

        for f in &self.info.files {
            let id =
                util::file_rpc_id(&self.info.hash, f.path.as_path().to_string_lossy().as_ref());
            r.push(resource::Resource::File(resource::File {
                id,
                torrent_id: self.rpc_id(),
                availability: 0.,
                progress: 0.,
                priority: 3,
                path: f.path.as_path().to_string_lossy().into_owned(),
            }))
        }

        r.push(resource::Resource::Tracker(resource::Tracker {
            id: util::trk_rpc_id(&self.info.hash, &self.info.announce),
            torrent_id: self.rpc_id(),
            url: self.info.announce.clone(),
            last_report: Utc::now(),
            error: None,
        }));

        r
    }

    pub fn send_rpc_removal(&mut self) {
        let mut r = Vec::new();
        r.push(self.rpc_id());
        for i in 0..self.info.pieces() {
            let id = util::piece_rpc_id(&self.info.hash, i as u64);
            r.push(id)
        }
        for f in &self.info.files {
            let id =
                util::file_rpc_id(&self.info.hash, f.path.as_path().to_string_lossy().as_ref());
            r.push(id)
        }
        r.push(util::trk_rpc_id(&self.info.hash, &self.info.announce));
        // TOOD: Tracker removal too
        self.cio.msg_rpc(rpc::CtlMessage::Removed(r));
    }

    fn error(&self) -> Option<String> {
        match self.status {
            Status::DiskError => Some("Disk error!".to_owned()),
            _ => None,
        }
    }

    fn sequential(&self) -> bool {
        self.picker.is_sequential()
    }

    fn progress(&self) -> f32 {
        self.pieces.iter().count() as f32 / self.info.pieces() as f32
    }

    fn availability(&self) -> f32 {
        // TODO: ??
        0.
    }

    pub fn reset_last_tx_rate(&mut self) -> (u64, u64) {
        let res = self.get_last_tx_rate();
        self.last_clear = Utc::now();
        self.last_ul = 0;
        self.last_dl = 0;
        res
    }

    // TODO: Implement Exp Moving Avg Somewhere
    pub fn get_last_tx_rate(&self) -> (u64, u64) {
        let dur = Utc::now()
            .signed_duration_since(self.last_clear)
            .num_milliseconds() as u64;
        let ul = (1000 * self.last_ul) / dur;
        let dl = (1000 * self.last_dl) / dur;
        (ul, dl)
    }

    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(&mut self, index: u32, begin: u32, data: Box<[u8; 16_384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        self.cio.msg_disk(disk::Request::write(
            self.id,
            data,
            locs,
            self.path.clone(),
        ));
    }

    /// Issues a read request of the given torrent
    fn request_read(&mut self, id: usize, index: u32, begin: u32, data: Box<[u8; 16_384]>) {
        let locs = self.info.block_disk_locs(index, begin);
        let len = self.info.block_len(index, begin);
        let ctx = disk::Ctx::new(id, self.id, index, begin, len);
        self.cio.msg_disk(disk::Request::read(
            ctx,
            data,
            locs,
            self.path.clone(),
        ));
    }

    fn make_requests_pid(&mut self, pid: usize) {
        let peer = self.peers.get_mut(&pid).expect(
            "Expected peer id not present",
        );
        if self.status.stopped() {
            return;
        }
        while peer.can_queue_req() {
            if let Some(block) = self.picker.pick(peer) {
                peer.request_piece(
                    block.index,
                    block.offset,
                    self.info.block_len(block.index, block.offset),
                );
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
            if let Some(block) = self.picker.pick(peer) {
                peer.request_piece(
                    block.index,
                    block.offset,
                    self.info.block_len(block.index, block.offset),
                );
            } else {
                break;
            }
        }
    }

    pub fn add_peer(&mut self, conn: PeerConn) -> Option<usize> {
        if let Ok(p) = Peer::new(conn, self, None, None) {
            let pid = p.id();
            trace!(self.l, "Adding peer {:?}!", pid);
            self.picker.add_peer(&p);
            self.peers.insert(pid, p);
            Some(pid)
        } else {
            None
        }
    }

    pub fn add_inc_peer(&mut self, conn: PeerConn, id: [u8; 20], rsv: [u8; 8]) -> Option<usize> {
        if let Ok(p) = Peer::new(conn, self, Some(id), Some(rsv)) {
            let pid = p.id();
            debug!(self.l, "Adding peer {:?}!", pid);
            self.picker.add_peer(&p);
            self.peers.insert(pid, p);
            Some(pid)
        } else {
            None
        }
    }

    fn set_status(&mut self, status: Status) {
        if self.status == status {
            return;
        }
        self.status = status;
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TorrentStatus {
                id,
                error: match status {
                    Status::DiskError => Some("Disk error".to_owned()),
                    _ => None,
                },
                status: status.into(),
            },
        ]));
    }

    pub fn update_rpc_peers(&mut self) {
        let availability = self.availability();
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TorrentPeers {
                id,
                peers: self.peers.len() as u16,
                availability,
            },
        ]));
    }

    pub fn update_rpc_tracker(&mut self) {
        let id = util::trk_rpc_id(&self.info.hash, &self.info.announce);
        let error = match self.tracker {
            TrackerStatus::Failure(ref r) => Some(r.clone()),
            TrackerStatus::Error => Some(
                "Failed to query tracker for an unknown reason.".to_owned(),
            ),
            _ => None,
        };
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TrackerStatus {
                id,
                last_report: Utc::now(),
                error,
            },
        ]));
    }

    pub fn update_rpc_transfer(&mut self) {
        let progress = self.progress();
        let (rate_up, rate_down) = self.get_last_tx_rate();
        let id = self.rpc_id();
        let mut updates = Vec::new();
        updates.push(SResourceUpdate::TorrentTransfer {
            id,
            rate_up,
            rate_down,
            transferred_up: self.uploaded,
            transferred_down: self.downloaded,
            progress,
        });
        if !self.status.leeching() {
            for pid in self.choker.unchoked().iter() {
                if let Some(p) = self.peers.get_mut(pid) {
                    let (rate_up, rate_down) = p.get_tx_rates();
                    updates.push(SResourceUpdate::Rate {
                        id: util::peer_rpc_id(&self.info.hash, *pid as u64),
                        rate_up,
                        rate_down,
                    });
                }
            }
        } else {
            for (pid, p) in &mut self.peers {
                if p.remote_status().choked || !p.ready() {
                    continue;
                }
                let (rate_up, rate_down) = p.get_tx_rates();
                let id = util::peer_rpc_id(&self.info.hash, *pid as u64);
                updates.push(SResourceUpdate::Rate {
                    id,
                    rate_up,
                    rate_down,
                });
            }
        }
        let mut files = HashMap::new();
        for p in self.pieces.iter() {
            for loc in self.info.piece_disk_locs(p as u32) {
                if !files.contains_key(&loc.file) {
                    files.insert(loc.file.clone(), (0, 0));
                }
                files.get_mut(&loc.file).unwrap().0 += loc.end - loc.start;
            }
        }
        for f in &self.info.files {
            files.get_mut(&f.path).map(|v| v.1 = f.length);
        }
        for (p, d) in files {
            let id = util::file_rpc_id(&self.info.hash, p.as_path().to_string_lossy().as_ref());
            updates.push(SResourceUpdate::FileProgress {
                id,
                progress: (d.0 as f32 / d.1 as f32),
            });
        }
        self.cio.msg_rpc(rpc::CtlMessage::Update(updates));
    }

    fn cleanup_peer(&mut self, peer: &mut Peer<T>) {
        trace!(self.l, "Removing {:?}!", peer);
        self.choker.remove_peer(peer, &mut self.peers);
        self.leechers.remove(&peer.id());
        self.picker.remove_peer(peer);
    }

    pub fn pause(&mut self) {
        debug!(self.l, "Pausing torrent!");
        match self.status {
            Status::Paused => {}
            _ => {
                debug!(self.l, "Sending stopped request to trk");
                let req = tracker::Request::stopped(self);
                self.cio.msg_trk(req);
            }
        }
        self.set_status(Status::Paused);
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
                    self.validate();
                } else {
                    self.request_all();
                    self.set_status(Status::Idle);
                }
            }
            _ => {}
        }
        if self.pieces.complete() {
            self.set_status(Status::Idle);
        } else {
            self.set_status(Status::Pending);
        }
    }

    fn validate(&mut self) {
        self.cio.msg_disk(
            disk::Request::validate(self.id, self.info.clone()),
        );
        self.set_status(Status::Validating);
    }

    fn request_all(&mut self) {
        for pid in self.pids() {
            self.make_requests_pid(pid);
        }
    }

    fn pids(&self) -> Vec<usize> {
        self.peers.keys().cloned().collect()
    }

    pub fn change_picker(&mut self, sequential: bool) {
        debug!(self.l, "Swapping pickers!");
        self.picker.change_picker(sequential);
        for peer in self.peers.values() {
            self.picker.add_peer(peer);
        }
        let id = self.rpc_id();
        let sequential = self.picker.is_sequential();
        self.cio.msg_rpc(rpc::CtlMessage::Update(
            vec![SResourceUpdate::TorrentPicker { id, sequential }],
        ));
    }
}

impl<T: cio::CIO> fmt::Debug for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {{ info: {:?} }}", self.info)
    }
}

impl<T: cio::CIO> fmt::Display for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent {}", util::hash_to_id(&self.info.hash[..]))
    }
}

impl<T: cio::CIO> Drop for Torrent<T> {
    fn drop(&mut self) {
        debug!(self.l, "Removing peers");
        for (id, peer) in self.peers.drain() {
            trace!(self.l, "Removing peer {:?}", peer);
            self.leechers.remove(&id);
        }
        match self.status {
            Status::Paused => {}
            _ => {
                let req = tracker::Request::stopped(self);
                self.cio.msg_trk(req);
            }
        }
        self.send_rpc_removal();
    }
}

impl Into<rpc::resource::Status> for Status {
    fn into(self) -> rpc::resource::Status {
        match self {
            Status::Pending => rpc::resource::Status::Pending,
            Status::Paused => rpc::resource::Status::Paused,
            Status::Idle => rpc::resource::Status::Idle,
            Status::Leeching => rpc::resource::Status::Leeching,
            Status::Seeding => rpc::resource::Status::Seeding,
            Status::Validating => rpc::resource::Status::Hashing,
            Status::DiskError => rpc::resource::Status::Error,
        }
    }
}
