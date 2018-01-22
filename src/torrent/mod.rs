pub mod info;
pub mod peer;
pub mod bitfield;
mod picker;
mod choker;

use std::fmt;
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::borrow::Cow;

use bincode;
use chrono::{DateTime, Utc};
use url::Url;

pub use self::bitfield::Bitfield;
pub use self::info::{Info, LocIter};
pub use self::peer::{Peer, PeerConn};
pub use self::peer::Message;
pub use self::picker::Block;

use self::picker::Picker;
use {bencode, disk, rpc, util, CONFIG, EXT_PROTO, UT_META_ID};
use control::cio;
use rpc::resource::{self, Resource, SResourceUpdate};
use throttle::Throttle;
use tracker::{self, TrackerResponse};
use util::{AView, FHashSet, MHashMap, UHashMap};
use session::torrent::current::Session;
use {session, stat};

#[derive(Clone, Debug, PartialEq)]
pub enum TrackerStatus {
    Updating,
    Ok {
        seeders: u32,
        leechers: u32,
        interval: u32,
    },
    Failure(String),
}

pub struct Torrent<T: cio::CIO> {
    id: usize,
    pieces: Bitfield,
    info: Arc<Info>,
    cio: T,
    uploaded: u64,
    downloaded: u64,
    stat: stat::EMA,
    priority: u8,
    priorities: Vec<u8>,
    throttle: Throttle,
    trackers: VecDeque<Tracker>,
    peers: UHashMap<Peer<T>>,
    leechers: FHashSet<usize>,
    picker: Picker,
    status: Status,
    choker: choker::Choker,
    dirty: bool,
    path: Option<String>,
    info_bytes: Vec<u8>,
    info_idx: Option<usize>,
    created: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub paused: bool,
    pub validating: bool,
    pub error: Option<String>,
    pub state: StatusState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusState {
    Magnet,
    // Torrent has not acquired all pieces
    Incomplete,
    // Torrent has acquired all pieces, regardless of validity
    Complete,
}

pub struct Tracker {
    pub url: AView<Url>,
    pub status: TrackerStatus,
    pub last_announce: DateTime<Utc>,
    pub update: Option<Instant>,
}

impl Status {
    pub fn leeching(&self) -> bool {
        match self.state {
            StatusState::Incomplete => true,
            _ => false,
        }
    }

    pub fn stopped(&self) -> bool {
        self.paused || self.error.is_some()
    }

    pub fn completed(&self) -> bool {
        match self.state {
            StatusState::Complete => true,
            _ => false,
        }
    }

    pub fn as_rpc(&self, ul: u64, dl: u64) -> rpc::resource::Status {
        if self.paused {
            return rpc::resource::Status::Paused;
        }
        if self.validating {
            return rpc::resource::Status::Hashing;
        }
        if self.error.is_some() {
            return rpc::resource::Status::Error;
        }

        match self.state {
            StatusState::Incomplete => {
                if dl == 0 {
                    rpc::resource::Status::Pending
                } else {
                    rpc::resource::Status::Leeching
                }
            }
            StatusState::Complete => {
                if ul == 0 {
                    rpc::resource::Status::Idle
                } else {
                    rpc::resource::Status::Seeding
                }
            }
            StatusState::Magnet => rpc::resource::Status::Magnet,
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
        start: bool,
    ) -> Torrent<T> {
        debug!("Creating {:?}", info);
        let peers = UHashMap::default();
        let pieces = Bitfield::new(u64::from(info.pieces()));
        let leechers = FHashSet::default();
        let mut status = Status {
            paused: !start,
            validating: false,
            error: None,
            state: StatusState::Incomplete,
        };
        let priorities = vec![3; info.files.len()];
        let info_idx = if info.complete() {
            None
        } else {
            status.state = StatusState::Magnet;
            Some(::std::usize::MAX)
        };
        let info_bytes = if info_idx.is_none() {
            info.to_bencode().encode_to_buf()
        } else {
            vec![]
        };
        let info = Arc::new(info);
        let picker = Picker::new(&info, &pieces, &priorities);

        let mut trackers = VecDeque::with_capacity(1);
        if !info.url_list.is_empty() {
            for (i, list) in info.url_list.iter().enumerate() {
                for (j, _) in list.iter().enumerate() {
                    let tracker = Tracker {
                        status: TrackerStatus::Updating,
                        update: None,
                        last_announce: Utc::now(),
                        url: AView::new(&info, |inf| &inf.url_list[i][j]),
                    };
                    trackers.push_back(tracker);
                }
            }
        } else if info.announce.is_some() {
            let tracker = Tracker {
                status: TrackerStatus::Updating,
                update: None,
                last_announce: Utc::now(),
                url: AView::new(&info, |i| i.announce.as_ref().unwrap()),
            };
            trackers.push_back(tracker);
        }

        let mut t = Torrent {
            id,
            info,
            path,
            peers,
            pieces,
            picker,
            priority: 3,
            priorities,
            uploaded: 0,
            downloaded: 0,
            stat: stat::EMA::new(),
            cio,
            leechers,
            throttle,
            trackers,
            choker: choker::Choker::new(),
            dirty: true,
            status: status.clone(),
            info_bytes,
            info_idx,
            created: Utc::now(),
        };
        t.start();
        if CONFIG.disk.validate && t.info_idx.is_none() {
            t.validate();
        } else {
            t.announce_start();
            t.announce_status();
        }
        t
    }

    pub fn deserialize(
        id: usize,
        data: &[u8],
        mut throttle: Throttle,
        cio: T,
    ) -> Option<Torrent<T>> {
        let d = if let Some(d) = session::torrent::load(data) {
            d
        } else {
            return None;
        };
        debug!("Torrent data deserialized!");
        let peers = UHashMap::default();
        let leechers = FHashSet::default();

        let info = Arc::new(Info {
            name: d.info.name,
            announce: d.info.announce.and_then(|u| Url::parse(&u).ok()),
            piece_len: d.info.piece_len,
            total_len: d.info.total_len,
            hashes: d.info.hashes,
            hash: d.info.hash,
            files: d.info
                .files
                .into_iter()
                .map(|f| info::File {
                    path: f.path,
                    length: f.length,
                })
                .collect(),
            private: d.info.private,
            be_name: d.info.be_name,
            piece_idx: d.info.piece_idx,
            url_list: vec![],
        });

        let info_idx = if info.complete() {
            None
        } else {
            Some(::std::usize::MAX)
        };
        let info_bytes = if info_idx.is_none() {
            info.to_bencode().encode_to_buf()
        } else {
            vec![]
        };
        let picker = picker::Picker::new(&info, &d.pieces, &d.priorities);
        throttle.set_ul_rate(d.throttle_ul);
        throttle.set_dl_rate(d.throttle_dl);

        let mut trackers = VecDeque::with_capacity(1);
        if info.announce.is_some() {
            let tracker = Tracker {
                status: TrackerStatus::Updating,
                update: None,
                last_announce: Utc::now(),
                url: AView::new(&info, |i| i.announce.as_ref().unwrap()),
            };
            trackers.push_back(tracker);
        }

        let mut t = Torrent {
            id,
            info,
            peers,
            pieces: d.pieces,
            picker,
            uploaded: d.uploaded,
            downloaded: d.downloaded,
            stat: stat::EMA::new(),
            priorities: d.priorities,
            priority: d.priority,
            cio,
            leechers,
            throttle,
            trackers,
            choker: choker::Choker::new(),
            dirty: false,
            status: Status {
                paused: d.status.paused,
                validating: d.status.validating,
                error: d.status.error,
                state: match d.status.state {
                    session::torrent::current::StatusState::Magnet => StatusState::Magnet,
                    session::torrent::current::StatusState::Incomplete => StatusState::Incomplete,
                    session::torrent::current::StatusState::Complete => StatusState::Complete,
                },
            },
            path: d.path,
            info_bytes,
            info_idx,
            created: d.created,
        };
        t.status.error = None;
        t.status.validating = false;
        t.start();
        t.announce_start();
        Some(t)
    }

    pub fn serialize(&mut self) {
        let d = Session {
            info: session::torrent::current::Info {
                name: self.info.name.clone(),
                announce: self.info.announce.as_ref().map(|a| a.as_str().to_owned()),
                piece_len: self.info.piece_len,
                total_len: self.info.total_len,
                hashes: self.info.hashes.clone(),
                hash: self.info.hash,
                files: self.info
                    .files
                    .iter()
                    .cloned()
                    .map(|f| session::torrent::current::File {
                        path: f.path,
                        length: f.length,
                    })
                    .collect(),
                private: self.info.private,
                be_name: self.info.be_name.clone(),
                piece_idx: self.info.piece_idx.clone(),
            },
            pieces: self.pieces.clone(),
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            status: session::torrent::current::Status {
                paused: self.status.paused,
                validating: self.status.validating,
                error: self.status.error.clone(),
                state: match self.status.state {
                    StatusState::Magnet => session::torrent::current::StatusState::Magnet,
                    StatusState::Incomplete => session::torrent::current::StatusState::Incomplete,
                    StatusState::Complete => session::torrent::current::StatusState::Complete,
                },
            },
            path: self.path.clone(),
            priorities: self.priorities.clone(),
            priority: self.priority,
            created: self.created,
            throttle_ul: self.throttle.ul_rate(),
            throttle_dl: self.throttle.dl_rate(),
        };
        let data = bincode::serialize(&d, bincode::Infinite).expect("Serialization failed!");
        debug!("Sending serialization request!");
        self.cio
            .msg_disk(disk::Request::serialize(self.id, data, self.info.hash));
        self.dirty = false;
    }

    pub fn rpc_id(&self) -> String {
        util::hash_to_id(&self.info.hash[..])
    }

    pub fn delete(&mut self, artifacts: bool) {
        debug!("Sending file deletion request!");
        let mut files = Vec::new();
        for file in &self.info.files {
            files.push(file.path.clone());
        }
        self.cio.msg_disk(disk::Request::delete(
            self.id,
            self.info.hash,
            files,
            self.path.clone(),
            artifacts,
        ));
    }

    pub fn pieces(&self) -> &Bitfield {
        &self.pieces
    }

    pub fn set_tracker_response(&mut self, url: &Url, resp: &tracker::Result<TrackerResponse>) {
        debug!("Processing tracker response");
        let mut time = Instant::now();
        match *resp {
            Ok(ref r) => {
                self.trackers
                    .iter_mut()
                    .find(|t| &*t.url == url)
                    .map(|tracker| {
                        time += Duration::from_secs(u64::from(r.interval));
                        tracker.status = TrackerStatus::Ok {
                            seeders: r.seeders,
                            leechers: r.leechers,
                            interval: r.interval,
                        };
                        tracker.update = Some(time);
                        tracker.last_announce = Utc::now();
                    });
            }
            Err(tracker::Error(tracker::ErrorKind::TrackerError(ref s), _)) => {
                self.trackers
                    .iter_mut()
                    .find(|t| &*t.url == url)
                    .map(|tracker| {
                        time += Duration::from_secs(300);
                        tracker.update = Some(time);
                        tracker.status = TrackerStatus::Failure(s.clone());
                        tracker.last_announce = Utc::now();
                    });
            }
            Err(ref e) => {
                self.trackers
                    .iter_mut()
                    .find(|t| &*t.url == url)
                    .map(|tracker| {
                        error!("Failed to query tracker {}: {}", url, e);
                        // Wait 5 minutes before trying again
                        time += Duration::from_secs(300);
                        tracker.update = Some(time);
                        let reason = format!("Couldn't contact tracker: {}", e);
                        tracker.status = TrackerStatus::Failure(reason);
                        tracker.last_announce = Utc::now();
                    });
            }
        }

        if resp.is_err() && self.trackers.iter().find(|t| &*t.url == url).is_some() {
            if let Some(front) = self.trackers.pop_front() {
                self.trackers.push_back(front);
                self.update_tracker();
            }
        }
        self.update_rpc_tracker();
    }

    pub fn try_update_tracker(&mut self) {
        if let Some(end) = self.trackers.front().and_then(|t| t.update) {
            debug!("Updating tracker at interval!");
            let cur = Instant::now();
            if cur >= end {
                self.update_tracker();
            }
        } else {
            self.update_tracker();
        }
    }

    fn update_tracker(&mut self) {
        if let Some(req) = tracker::Request::interval(self) {
            self.cio.msg_trk(req);
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

    pub fn remove_tracker(&mut self, rpc_id: &str) {
        let ih = &self.info.hash;
        self.trackers
            .retain(|trk| util::trk_rpc_id(ih, trk.url.as_str()) != rpc_id)
    }

    pub fn update_tracker_req(&mut self, rpc_id: &str) {
        self.trackers
            .iter()
            .find(|trk| util::trk_rpc_id(&self.info.hash, trk.url.as_str()) == rpc_id)
            .and_then(|trk| tracker::Request::custom(self, trk.url.clone()))
            .map(|req| self.cio.msg_trk(req));
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

    pub fn uploaded(&self) -> u64 {
        self.uploaded
    }

    pub fn downloaded(&self) -> u64 {
        self.downloaded
    }

    pub fn info(&self) -> &Info {
        &self.info
    }

    pub fn trackers(&self) -> &VecDeque<Tracker> {
        &self.trackers
    }

    pub fn handle_disk_resp(&mut self, resp: disk::Response) {
        match resp {
            disk::Response::Read { context, data } => {
                trace!("Received piece from disk, uploading!");
                if let Some(peer) = self.peers.get_mut(&context.pid) {
                    let p = Message::s_piece(context.idx, context.begin, context.length, data);
                    // This may not be 100% accurate, but close enough for now.
                    self.uploaded += u64::from(context.length);
                    self.stat.add_ul(u64::from(context.length));
                    self.dirty = true;
                    peer.send_message(p);
                }
            }
            disk::Response::Moved { path, .. } => {
                debug!("Moved torrent!");
                let id = self.rpc_id();
                self.path = Some(path.clone());
                self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                    resource::SResourceUpdate::TorrentPath {
                        id,
                        kind: resource::ResourceKind::Torrent,
                        path,
                    },
                ]));
            }
            disk::Response::ValidationComplete { mut invalid, .. } => {
                debug!("Validation completed!");
                self.status.validating = false;
                // Ignore invalid pieces which are
                // part of an invalid file(none of the disk locations
                // refer to files which aren't being downloaded(pri. 1)
                invalid.retain(|i| {
                    Info::piece_disk_locs(&self.info, *i).all(|loc| self.priorities[loc.file] != 0)
                });
                if invalid.is_empty() {
                    info!("Torrent succesfully downloaded!");
                    if !self.complete() {
                        for i in 0..self.pieces.len() {
                            let complete = Info::piece_disk_locs(&self.info, i as u32)
                                .all(|loc| self.priorities[loc.file] != 0);
                            if complete {
                                self.pieces.set_bit(i);
                            }
                        }
                    }
                    self.set_finished();
                } else {
                    // If this is an initialization hash, start the torrent
                    // immediatly.
                    if !self.complete() {
                        debug!("initial validation complete, starting torrent");
                        // If there was some partial completion,
                        // set the pieces appropriately, then reset the
                        // picker to use the new bitfield
                        if invalid.len() != self.pieces.len() as usize {
                            for i in 0..self.pieces.len() {
                                let complete = Info::piece_disk_locs(&self.info, i as u32)
                                    .all(|loc| self.priorities[loc.file] != 0);
                                if complete {
                                    self.pieces.set_bit(i);
                                }
                            }

                            for piece in invalid {
                                self.pieces.unset_bit(u64::from(piece));
                            }
                            let mut rpc_updates = vec![];
                            self.cio.msg_rpc(rpc::CtlMessage::Update(rpc_updates));
                        }
                        self.announce_start();
                    } else {
                        let mut rpc_updates = vec![];
                        for piece in invalid {
                            self.picker.invalidate_piece(piece);
                            self.pieces.unset_bit(u64::from(piece));
                        }
                        self.cio.msg_rpc(rpc::CtlMessage::Update(rpc_updates));
                        self.request_all();
                    }
                    self.status.state = StatusState::Incomplete;
                }
                // update the RPC stats once done
                self.update_rpc_transfer();
            }
            disk::Response::Error { err, .. } => {
                error!("Disk error: {:?}", err);
                self.status.error = Some(format!("{:?}", err));
            }
        }
        self.announce_status();
    }

    fn check_complete(&mut self) {
        let mut idx = 0;
        for piece in self.pieces.iter() {
            while idx != piece {
                if Info::piece_disk_locs(&self.info, piece as u32)
                    .all(|loc| self.priorities[loc.file] != 0)
                {
                    return;
                }
                idx += 1
            }
            idx += 1
        }

        self.status.state = StatusState::Complete;
        self.serialize();
        if CONFIG.disk.validate {
            debug!("Beginning validation");
            self.validate();
        } else {
            debug!("Torrent complete");
            self.set_finished();
        }
    }
    /// Signal that we've downloaded and verified the torrent
    fn set_finished(&mut self) {
        // It's ok to say we've completed even if we haven't downloaded everything since
        // the `left` field should indicate how much there still is to download.
        if let Some(req) = tracker::Request::completed(self) {
            self.cio.msg_trk(req);
        }
        // Order here is important, if we're in an idle status,
        // rpc updates don't occur.
        self.update_rpc_transfer();
        self.status.state = StatusState::Complete;
        self.announce_status();

        // Remove all seeding peers.
        let leechers = &self.leechers;
        let seeders = self.peers
            .iter()
            .filter(|&(id, _)| !leechers.contains(id))
            .map(|(id, _)| *id);
        for seeder in seeders {
            self.cio.remove_peer(seeder);
        }
    }

    pub fn peer_ev(&mut self, pid: cio::PID, evt: cio::Result<Message>) -> Result<(), ()> {
        // TODO: Consider Boxing peers so it's just pointer insert/removal
        let mut peer = self.peers.remove(&pid).ok_or(())?;
        match evt {
            Ok(mut msg) => {
                if peer.handle_msg(&mut msg).is_ok() && self.handle_msg(msg, &mut peer).is_ok() {
                    self.peers.insert(pid, peer);
                    return Ok(());
                } else {
                    // In order to ensure there's only one source of truth,
                    // we instruct the CIO to remove the peer and let the event bubble up.
                    // We then will receieve it later and call cleanup_peer as needed.
                    // This ensures that events flow from control -> torrent and get
                    // properly processed
                    self.cio.remove_peer(pid);
                    self.peers.insert(pid, peer);
                    return Ok(());
                }
            }
            Err(e) => {
                debug!(
                    "Removing peer {}, {}",
                    util::peer_rpc_id(&self.info.hash, pid as u64),
                    e
                );
                self.cleanup_peer(&mut peer);
            }
        }
        Err(())
    }

    pub fn handle_msg(&mut self, msg: Message, peer: &mut Peer<T>) -> Result<(), ()> {
        trace!("Received {:?} from peer", msg);
        match msg {
            Message::Handshake { rsv, .. } => {
                if (rsv[EXT_PROTO.0] & EXT_PROTO.1) != 0 {
                    let mut ed = BTreeMap::new();
                    let mut m = BTreeMap::new();
                    m.insert(
                        "ut_metadata".to_owned(),
                        bencode::BEncode::Int(i64::from(UT_META_ID)),
                    );
                    ed.insert("m".to_owned(), bencode::BEncode::Dict(m));
                    ed.insert(
                        "metadata_size".to_owned(),
                        bencode::BEncode::Int(self.info_bytes.len() as i64),
                    );
                    let payload = bencode::BEncode::Dict(ed).encode_to_buf();
                    peer.send_message(Message::Extension { id: 0, payload });
                }
            }
            Message::Extension { id, payload } => {
                self.handle_ext(id, payload, peer)?;
            }
            Message::Bitfield(_) => {
                if self.pieces.usable(peer.pieces()) {
                    peer.interested();
                }
                if self.info.complete() {
                    self.picker.add_peer(peer);
                }
                if !peer.pieces().complete() {
                    self.leechers.insert(peer.id());
                } else if self.complete() {
                    // Don't waste a connection on a peer if they're also a seeder
                    return Err(());
                }
            }
            Message::Have(idx) => {
                if self.info.complete() {
                    self.picker.piece_available(idx);
                }
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
                if !self.status.stopped() && self.info.complete() {
                    Torrent::make_requests(peer, &mut self.picker, &self.info);
                }
            }
            Message::Piece {
                index,
                begin,
                data,
                length,
            } => {
                // Ignore a piece we already have, this could happen from endgame
                if self.pieces.has_bit(u64::from(index)) {
                    return Ok(());
                }

                // Even though we have the data, if we are stopped we shouldn't use the disk
                // regardless.
                if self.status.stopped() {
                    return Ok(());
                }

                // The length doesn't match what it should be
                if self.info.block_len(index, begin) != length {
                    return Err(());
                }

                // We already have this block, don't do anything with it, could happen
                // from endgame
                if self.picker.have_block(Block::new(index, begin)) {
                    return Ok(());
                }

                let pr = self.picker.completed(Block::new(index, begin));
                let (piece_done, peers) = if let Ok(r) = pr {
                    r
                } else {
                    return Ok(());
                };

                self.dirty = true;
                self.write_piece(index, begin, data);

                self.downloaded += u64::from(length);
                self.stat.add_dl(u64::from(length));

                if piece_done {
                    self.pieces.set_bit(u64::from(index));

                    // Begin validation, and save state if the torrent is done
                    self.check_complete();

                    // Tell all relevant peers we got the piece
                    let m = Message::Have(index);
                    for pid in &self.leechers {
                        if let Some(peer) = self.peers.get_mut(pid) {
                            if !peer.pieces().has_bit(u64::from(index)) {
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

                if !self.complete() && !self.status.stopped() {
                    Torrent::make_requests(peer, &mut self.picker, &self.info);
                }
            }
            Message::Request {
                index,
                begin,
                length,
            } => {
                if !self.status.stopped() && !self.status.leeching() {
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
            Message::KeepAlive | Message::Choke | Message::Cancel { .. } | Message::Port(_) => {}

            Message::SharedPiece { .. } => unreachable!(),
        }
        Ok(())
    }

    fn handle_ext(&mut self, id: u8, payload: Vec<u8>, peer: &mut Peer<T>) -> Result<(), ()> {
        if id == 0 {
            let b = bencode::decode_buf(&payload).map_err(|_| ())?;
            let mut d = b.into_dict().ok_or(())?;
            let m = d.remove("m").and_then(|v| v.into_dict()).ok_or(())?;
            if m.contains_key("ut_metadata") {
                let size = d.remove("metadata_size")
                    .and_then(|v| v.into_int())
                    .ok_or(())?;
                if let Some(::std::usize::MAX) = self.info_idx {
                    if size % 16_384 == 0 {
                        self.info_idx = Some(size as usize / 16_384 - 1);
                    } else {
                        self.info_idx = Some(size as usize / 16_384);
                    }
                    self.info_bytes.reserve(size as usize);
                    unsafe {
                        self.info_bytes.set_len(size as usize);
                    }
                }
                if !self.info.complete() {
                    // Request the first index chunk to see if they have it
                    let mut respb = BTreeMap::new();
                    respb.insert("msg_type".to_owned(), bencode::BEncode::Int(0));
                    respb.insert("piece".to_owned(), bencode::BEncode::Int(0));
                    let payload = bencode::BEncode::Dict(respb).encode_to_buf();
                    let utm_id = if let Some(i) = peer.exts().ut_meta {
                        i
                    } else {
                        return Err(());
                    };
                    peer.send_message(Message::Extension {
                        id: utm_id,
                        payload,
                    });
                }
            }
        } else if id == UT_META_ID {
            let utm_id = if let Some(i) = peer.exts().ut_meta {
                i
            } else {
                return Err(());
            };
            let b = bencode::decode_buf(&payload).map_err(|_| ())?;
            let mut d = b.into_dict().ok_or(())?;
            let t = d.remove("msg_type").and_then(|v| v.into_int()).ok_or(())?;
            let p = d.remove("piece").and_then(|v| v.into_int()).ok_or(())? as usize;
            if p * 16_384 >= self.info_bytes.len() {
                return Err(());
            }
            // Our metadata request strategy is as follows: after requesting the first
            // index chunk, we attempt to request every single subsequent chunk from
            // a peer which responds succesfully. This is slightly wasteful, but
            // simplifies logic (since we don't have to do "index piece picking").
            match t {
                0 => {
                    let mut respb = BTreeMap::new();
                    if self.info_idx.is_none() {
                        respb.insert("msg_type".to_owned(), bencode::BEncode::Int(1));
                        respb.insert("piece".to_owned(), bencode::BEncode::Int(p as i64));
                        let size = if self.info_bytes.len() / 16_384 == p {
                            self.info_bytes.len() % 16_384
                        } else {
                            16_384
                        };
                        let total_size = self.info_bytes.len() as i64;
                        respb.insert("total_size".to_owned(), bencode::BEncode::Int(total_size));
                        let mut payload = bencode::BEncode::Dict(respb).encode_to_buf();
                        let s = p * 16_384;
                        payload.extend_from_slice(&self.info_bytes[s..s + size]);
                        peer.send_message(Message::Extension {
                            id: utm_id,
                            payload,
                        });
                    } else {
                        respb.insert("msg_type".to_owned(), bencode::BEncode::Int(2));
                        respb.insert("piece".to_owned(), bencode::BEncode::Int(p as i64));
                        let payload = bencode::BEncode::Dict(respb).encode_to_buf();
                        peer.send_message(Message::Extension {
                            id: utm_id,
                            payload,
                        });
                    }
                }
                1 => {
                    if let Some(idx) = self.info_idx {
                        let data_idx = util::find_subseq(&payload[..], b"ee").unwrap() + 2;
                        if payload.len() - data_idx > self.info_bytes.len() - p * 16_384 {
                            debug!(
                                "Metadata bounds invalid, goes to: {}, ibl: {}",
                                payload.len() - data_idx,
                                self.info_bytes.len() - p * 16_384,
                            );
                            return Err(());
                        }
                        let size = if p == idx {
                            self.info_bytes.len() - p * 16_384
                        } else {
                            16_384
                        };
                        (&mut self.info_bytes[p * 16_384..p * 16_384 + size])
                            .copy_from_slice(&payload[data_idx..]);
                        if p == idx {
                            let mut b = BTreeMap::new();
                            let bni = bencode::decode_buf(&self.info_bytes).map_err(|_| ())?;
                            b.insert(
                                "announce".to_owned(),
                                bencode::BEncode::String(
                                    self.info
                                        .announce
                                        .as_ref()
                                        .map(|u| u.as_str())
                                        .unwrap_or("")
                                        .as_bytes()
                                        .to_vec(),
                                ),
                            );
                            b.insert("info".to_owned(), bni);
                            let ni = Info::from_bencode(bencode::BEncode::Dict(b)).map_err(|_| ())?;
                            if ni.hash == self.info.hash {
                                debug!("Magnet file acquired succesfully!");
                                self.info_idx = None;
                                self.info = Arc::new(ni);
                                self.magnet_complete();
                            } else {
                                return Err(());
                            }
                        } else if p == 0 {
                            for i in 1..(idx + 1) {
                                let mut respb = BTreeMap::new();
                                respb.insert("msg_type".to_owned(), bencode::BEncode::Int(0));
                                respb.insert("piece".to_owned(), bencode::BEncode::Int(i as i64));
                                let payload = bencode::BEncode::Dict(respb).encode_to_buf();
                                peer.send_message(Message::Extension {
                                    id: utm_id,
                                    payload,
                                });
                            }
                        }
                    }
                }
                2 => {}
                i => {
                    debug!("Got unknown ut_meta id: {}", i);
                }
            }
        } else {
            debug!("Got unknown extension id: {}", id);
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
        if u.throttle_up.is_some() || u.throttle_down.is_some() {
            let tu = u.throttle_up.unwrap_or_else(|| self.throttle.ul_rate());
            let td = u.throttle_down.unwrap_or_else(|| self.throttle.dl_rate());
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
        for (i, f) in self.info.files.iter().enumerate() {
            let fid =
                util::file_rpc_id(&self.info.hash, f.path.as_path().to_string_lossy().as_ref());
            if fid == id {
                self.priorities[i] = priority;
            }
        }

        self.picker.set_priorities(&self.priorities, &self.info);

        self.check_complete();

        self.dirty = true;

        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::FilePriority {
                id,
                kind: resource::ResourceKind::File,
                priority,
            },
        ]));
    }

    fn start(&mut self) {
        debug!("Starting torrent");
        // Update RPC of the torrent, tracker, files, and peers
        let mut resources = Vec::new();
        resources.push(self.rpc_info());
        resources.extend(self.rpc_trk_info());
        if self.info_idx.is_none() {
            resources.extend(self.rpc_rel_info());
        }
        self.cio.msg_rpc(rpc::CtlMessage::Extant(resources));
        if self.info_idx.is_none() {
            self.update_rpc_transfer();
        }
        self.serialize();
    }

    fn announce_start(&mut self) {
        if self.status.stopped() {
            return;
        }
        if let Some(req) = tracker::Request::started(self) {
            self.cio.msg_trk(req);
        }
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
        self.status.completed()
    }

    fn set_throttle(&mut self, ul: Option<i64>, dl: Option<i64>) {
        self.throttle.set_ul_rate(ul);
        self.throttle.set_dl_rate(dl);
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::Throttle {
                id,
                kind: resource::ResourceKind::Torrent,
                throttle_up: ul,
                throttle_down: dl,
            },
        ]));
    }

    fn magnet_complete(&mut self) {
        self.status.state = StatusState::Incomplete;
        self.announce_status();
        self.pieces = Bitfield::new(u64::from(self.info.pieces()));
        self.priorities = vec![3; self.info.files.len()];
        for peer in self.peers.values_mut() {
            peer.magnet_complete(&self.info);
        }

        let resources = self.rpc_rel_info();
        self.cio.msg_rpc(rpc::CtlMessage::Extant(resources));
        let update = self.rpc_info();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::Resource(Cow::Owned(update)),
        ]));
        self.serialize();

        let seq = self.picker.is_sequential();
        self.picker = Picker::new(&self.info, &self.pieces, &self.priorities);
        self.change_picker(seq);
        self.validate();
    }

    fn set_path(&mut self, path: String) {
        let from = if let Some(ref p) = self.path {
            p.clone()
        } else {
            CONFIG.disk.directory.clone()
        };
        self.cio.msg_disk(disk::Request::Move {
            tid: self.id,
            from,
            to: path,
            target: self.info.name.clone(),
        });
    }

    fn set_priority(&mut self, priority: u8) {
        // TODO: Implement priority somewhere(throttle or ctrl)
        self.priority = priority;
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::TorrentPriority {
                id,
                kind: resource::ResourceKind::Torrent,
                priority,
            },
        ]));
    }

    fn rpc_info(&self) -> resource::Resource {
        let (name, size, pieces, piece_size, files) = if self.info_idx.is_none() {
            (
                Some(self.info.name.clone()),
                Some(self.info.total_len),
                Some(u64::from(self.info.pieces())),
                Some(self.info.piece_len),
                Some(self.info.files.len() as u32),
            )
        } else {
            let name = if self.info.name == "" {
                None
            } else {
                Some(self.info.name.clone())
            };
            (name, None, None, None, None)
        };
        Resource::Torrent(resource::Torrent {
            id: self.rpc_id(),
            name,
            size,
            // TODO: Properly add this
            path: self.path.as_ref().unwrap_or(&CONFIG.disk.directory).clone(),
            created: self.created,
            modified: Utc::now(),
            status: self.status.as_rpc(self.stat.avg_ul(), self.stat.avg_dl()),
            error: self.error(),
            priority: self.priority,
            progress: self.progress(),
            availability: self.availability(),
            sequential: self.sequential(),
            rate_up: 0,
            rate_down: 0,
            throttle_up: self.throttle.ul_rate(),
            throttle_down: self.throttle.dl_rate(),
            transferred_up: self.uploaded,
            transferred_down: self.downloaded,
            peers: 0,
            trackers: self.trackers.len() as u8,
            pieces,
            piece_size,
            files,
            ..Default::default()
        })
    }

    fn rpc_rel_info(&self) -> Vec<resource::Resource> {
        let mut r = Vec::new();
        let mut files = Vec::new();
        for f in self.info.files.iter() {
            files.push((0, f.length));
        }

        for p in self.pieces.iter() {
            for loc in Info::piece_disk_locs(&self.info, p as u32) {
                files[loc.file].0 += loc.end - loc.start;
            }
        }

        for (i, (done, total)) in files.into_iter().enumerate() {
            let id = util::file_rpc_id(
                &self.info.hash,
                self.info.files[i].path.to_string_lossy().as_ref(),
            );
            let progress = if self.priorities[i] != 0 {
                done as f32 / total as f32
            } else {
                0.
            };
            r.push(resource::Resource::File(resource::File {
                id,
                torrent_id: self.rpc_id(),
                availability: 0.,
                progress,
                priority: self.priorities[i],
                path: self.info.files[i].path.to_string_lossy().into_owned(),
                size: total,
                ..Default::default()
            }))
        }

        r
    }

    fn rpc_trk_info(&self) -> Vec<resource::Resource> {
        self.trackers
            .iter()
            .map(|trk| {
                resource::Resource::Tracker(resource::Tracker {
                    id: util::trk_rpc_id(&self.info.hash, trk.url.as_str()),
                    torrent_id: self.rpc_id(),
                    url: Some(trk.url.as_ref().clone()),
                    last_report: trk.last_announce.clone(),
                    error: None,
                    ..Default::default()
                })
            })
            .collect()
    }

    pub fn send_rpc_removal(&mut self) {
        let mut r = Vec::new();
        r.push(self.rpc_id());
        for f in &self.info.files {
            let id =
                util::file_rpc_id(&self.info.hash, f.path.as_path().to_string_lossy().as_ref());
            r.push(id)
        }
        r.push(util::trk_rpc_id(
            &self.info.hash,
            self.info
                .announce
                .as_ref()
                .map(|u| u.as_str())
                .unwrap_or(""),
        ));
        // TODO: Tracker removal too
        self.cio.msg_rpc(rpc::CtlMessage::Removed(r));
    }

    fn error(&self) -> Option<String> {
        self.status.error.clone()
    }

    fn sequential(&self) -> bool {
        self.picker.is_sequential()
    }

    fn progress(&self) -> f32 {
        self.pieces.iter().count() as f32 / self.info.pieces() as f32
    }

    fn availability(&self) -> f32 {
        let mut peers_have = FHashSet::default();
        for (_, peer) in &self.peers {
            for piece in peer.pieces().iter() {
                peers_have.insert(piece);
            }
            if peers_have.len() as u64 == self.pieces.len() {
                return 1.0;
            }
        }
        peers_have.len() as f32 / self.pieces.len() as f32
    }

    /// Resets the last upload/download statistics, adjusting the internal
    /// status if nothing has been uploaded/downloaded in the interval.
    pub fn tick(&mut self) -> bool {
        self.stat.tick();
        let mut active = self.stat.active();
        self.picker.tick();

        for (_, peer) in self.peers.iter_mut() {
            active |= peer.tick();
        }
        active
    }

    pub fn get_last_tx_rate(&self) -> (u64, u64) {
        (self.stat.avg_ul(), self.stat.avg_dl())
    }

    /// Writes a piece of torrent info, with piece index idx,
    /// piece offset begin, piece length of len, and data bytes.
    /// The disk send handle is also provided.
    fn write_piece(&mut self, index: u32, begin: u32, data: Box<[u8; 16_384]>) {
        let locs = Info::block_disk_locs(&self.info, index, begin);
        self.cio
            .msg_disk(disk::Request::write(self.id, data, locs, self.path.clone()));
    }

    /// Issues a read request of the given torrent
    fn request_read(&mut self, id: usize, index: u32, begin: u32, data: Box<[u8; 16_384]>) {
        let locs = Info::block_disk_locs(&self.info, index, begin);
        let len = self.info.block_len(index, begin);
        let ctx = disk::Ctx::new(id, self.id, index, begin, len);
        self.cio
            .msg_disk(disk::Request::read(ctx, data, locs, self.path.clone()));
    }

    fn make_requests_pid(&mut self, pid: usize) {
        if self.status.stopped() {
            return;
        }
        let peer = self.peers
            .get_mut(&pid)
            .expect("Expected peer id not present");
        Torrent::make_requests(peer, &mut self.picker, &self.info);
    }

    fn make_requests(peer: &mut Peer<T>, picker: &mut Picker, info: &Info) {
        if let Some(m) = peer.queue_reqs() {
            for _ in 0..m {
                if let Some(block) = picker.pick(peer) {
                    peer.request_piece(
                        block.index,
                        block.offset,
                        info.block_len(block.index, block.offset),
                    );
                } else {
                    break;
                }
            }
        }
    }

    pub fn add_peer(&mut self, conn: PeerConn) -> Option<usize> {
        if let Ok(p) = Peer::new(conn, self, None, None) {
            let pid = p.id();
            trace!("Adding peer {:?}!", pid);
            if self.info_idx.is_none() {
                self.picker.add_peer(&p);
            }
            self.peers.insert(pid, p);
            Some(pid)
        } else {
            None
        }
    }

    pub fn add_inc_peer(&mut self, conn: PeerConn, id: [u8; 20], rsv: [u8; 8]) -> Option<usize> {
        if let Ok(p) = Peer::new(conn, self, Some(id), Some(rsv)) {
            let pid = p.id();
            debug!("Adding peer {:?}!", pid);
            if self.info_idx.is_none() {
                self.picker.add_peer(&p);
            }
            self.peers.insert(pid, p);
            Some(pid)
        } else {
            None
        }
    }

    pub fn announce_status(&mut self) {
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TorrentStatus {
                id,
                kind: resource::ResourceKind::Torrent,
                error: self.status.error.clone(),
                status: self.status.as_rpc(self.stat.avg_ul(), self.stat.avg_dl()),
            },
        ]));
    }

    pub fn update_rpc_peers(&mut self) {
        let availability = self.availability();
        let id = self.rpc_id();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TorrentPeers {
                id,
                kind: resource::ResourceKind::Torrent,
                peers: self.peers.len() as u16,
                availability,
            },
        ]));
    }

    pub fn update_rpc_tracker(&mut self) {
        let updates = self.trackers
            .iter()
            .map(|tracker| {
                let id = util::trk_rpc_id(&self.info.hash, tracker.url.as_str());
                let error = match tracker.status {
                    TrackerStatus::Failure(ref r) => Some(r.clone()),
                    _ => None,
                };
                SResourceUpdate::TrackerStatus {
                    id,
                    kind: resource::ResourceKind::Tracker,
                    last_report: Utc::now(),
                    error,
                }
            })
            .collect();
        self.cio.msg_rpc(rpc::CtlMessage::Update(updates));
    }

    pub fn update_rpc_transfer(&mut self) {
        let progress = self.progress();
        let (rate_up, rate_down) = self.get_last_tx_rate();
        let id = self.rpc_id();
        let mut updates = Vec::new();
        updates.push(SResourceUpdate::TorrentTransfer {
            id: id.clone(),
            kind: resource::ResourceKind::Torrent,
            rate_up,
            rate_down,
            transferred_up: self.uploaded,
            transferred_down: self.downloaded,
            progress,
        });

        for (pid, p) in &mut self.peers {
            if !p.active() {
                continue;
            }
            let (rate_up, rate_down) = p.get_tx_rates();
            updates.push(SResourceUpdate::Rate {
                id: util::peer_rpc_id(&self.info.hash, *pid as u64),
                kind: resource::ResourceKind::Peer,
                rate_up,
                rate_down,
            });
        }

        if self.stat.active() {
            let mut files = MHashMap::default();
            for (i, f) in self.info.files.iter().enumerate() {
                if self.priorities[i] != 0 {
                    files.insert(f.path.clone(), (0, f.length));
                }
            }

            for p in self.pieces.iter() {
                for loc in Info::piece_disk_locs(&self.info, p as u32) {
                    if let Some(f) = files.get_mut(loc.path()) {
                        f.0 += loc.end - loc.start;
                    }
                }
            }

            for (p, d) in files {
                let id = util::file_rpc_id(&self.info.hash, p.as_path().to_string_lossy().as_ref());
                updates.push(SResourceUpdate::FileProgress {
                    id,
                    kind: resource::ResourceKind::File,
                    progress: (d.0 as f32 / d.1 as f32),
                });
            }
        }
        self.cio.msg_rpc(rpc::CtlMessage::Update(updates));
        self.announce_status();
    }

    fn cleanup_peer(&mut self, peer: &mut Peer<T>) {
        trace!("Removing {:?}!", peer);
        self.choker.remove_peer(peer, &mut self.peers);
        self.leechers.remove(&peer.id());
        if self.info.complete() {
            self.picker.remove_peer(peer);
        }
    }

    pub fn pause(&mut self) {
        debug!("Pausing torrent!");
        if !self.status.paused {
            debug!("Sending stopped request to trk");
            if let Some(req) = tracker::Request::stopped(self) {
                self.cio.msg_trk(req);
            }
            self.status.paused = true;
            self.announce_status();
        }
    }

    pub fn resume(&mut self) {
        debug!("Resuming torrent!");
        if self.status.error.is_some() || self.status.paused {
            if self.status.error.is_some() {
                self.status.error = None;
            }
            if self.status.paused {
                debug!("Sending started request to trk");
                if let Some(req) = tracker::Request::started(self) {
                    self.cio.msg_trk(req);
                }
                self.status.paused = false;
            }
            self.request_all();
            self.announce_status();
        }
    }

    pub fn validate(&mut self) {
        self.cio.msg_disk(disk::Request::validate(
            self.id,
            self.info.clone(),
            self.path.clone(),
        ));
        self.status.validating = true;
        self.announce_status();
    }

    fn request_all(&mut self) {
        if self.status.stopped() || self.info_idx.is_some() {
            return;
        }
        for pid in self.pids() {
            self.make_requests_pid(pid);
        }
    }

    fn pids(&self) -> Vec<usize> {
        self.peers.keys().cloned().collect()
    }

    pub fn change_picker(&mut self, sequential: bool) {
        debug!("Swapping pickers!");
        self.picker.change_picker(sequential);
        for peer in self.peers.values() {
            self.picker.add_peer(peer);
        }
        let id = self.rpc_id();
        let sequential = self.picker.is_sequential();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            SResourceUpdate::TorrentPicker {
                id,
                kind: resource::ResourceKind::Torrent,
                sequential,
            },
        ]));
    }

    /*
    fn should_pick(&self) -> Bitfield {
        let mut b = Bitfield::new(self.pieces.len());
        // Only DL pieces which we don't yet have, and are in wanted.
        for i in 0..self.pieces.len() {
            if self.pieces.has_bit(i) || !self.wanted.has_bit(i) {
                b.set_bit(i);
            }
        }
        b
    }
    */
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
        debug!("Removing peers");
        for (id, peer) in self.peers.drain() {
            trace!("Removing peer {:?}", peer);
            self.leechers.remove(&id);
        }
        if !self.status.paused {
            if let Some(msg) = tracker::Request::stopped(self) {
                self.cio.msg_trk(msg);
            }
        }
        self.send_rpc_removal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use control::cio::{test, CIO};
    use throttle::*;
}
