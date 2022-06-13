pub mod bitfield;
mod choker;
pub mod info;
pub mod peer;
mod picker;

use std::borrow::Cow;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bencode::BEncode;
use byteorder::{BigEndian, ByteOrder};
use chrono::{DateTime, Utc};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use url::Url;

pub use self::bitfield::Bitfield;
pub use self::info::{Info, LocIter};
pub use self::peer::Message;
pub use self::peer::{Peer, PeerConn};
pub use self::picker::Block;

use self::picker::Picker;
use crate::buffers::Buffer;
use crate::control::cio;
use crate::rpc::resource::{self, Resource, SResourceUpdate};
use crate::session::torrent::current::Session;
use crate::throttle::Throttle;
use crate::tracker::{self, TrackerResponse};
use crate::util::{FHashSet, UHashMap};
use crate::{bencode, disk, rpc, util, CONFIG, EXT_PROTO, UT_META_ID, UT_PEX_ID};
use crate::{session, stat};

const MAX_PEERS: usize = 50;

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
    validating: FHashSet<u32>,
    info: Arc<Info>,
    cio: T,
    uploaded: u64,
    downloaded: u64,
    wasted: u64,
    stat: stat::EMA,
    files: Files,
    priority: u8,
    priorities: Arc<Vec<u8>>,
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
    pub validating: Option<f32>,
    pub error: Option<String>,
    pub state: StatusState,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusState {
    Magnet,
    Import,
    Incomplete,
    Complete,
}

pub struct Tracker {
    pub url: Arc<Url>,
    pub status: TrackerStatus,
    pub last_announce: DateTime<Utc>,
    pub update: Option<Instant>,
}

struct Files {
    done: Vec<u64>,
    dirty: FHashSet<usize>,
}

impl Status {
    pub fn magnet(&self) -> bool {
        match self.state {
            StatusState::Magnet => true,
            _ => false,
        }
    }

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
            StatusState::Complete => self.validating.is_none(),
            _ => false,
        }
    }

    pub fn should_dl(&self) -> bool {
        self.leeching() && !self.stopped() && self.validating.is_none()
    }

    pub fn as_rpc(&self, ul: u64, dl: u64) -> rpc::resource::Status {
        if self.paused {
            return rpc::resource::Status::Paused;
        }
        if self.validating.is_some() {
            return rpc::resource::Status::Hashing;
        }
        if self.error.is_some() {
            return rpc::resource::Status::Error;
        }

        match self.state {
            StatusState::Incomplete | StatusState::Import => {
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

impl Files {
    fn new(info: &Arc<Info>, pieces: &Bitfield) -> Files {
        let mut f = Files {
            done: vec![0; info.files.len()],
            dirty: FHashSet::default(),
        };
        f.rebuild(info, pieces);
        f
    }

    fn rebuild(&mut self, info: &Arc<Info>, pieces: &Bitfield) {
        for amnt in &mut self.done {
            *amnt = 0;
        }

        for p in pieces.iter() {
            for loc in Info::piece_disk_locs(info, p as u32) {
                self.done[loc.file] += (loc.end - loc.start) as u64;
            }
        }

        for i in 0..self.done.len() {
            self.dirty.insert(i);
        }
    }

    fn update(&mut self, info: &Arc<Info>, piece: u32) {
        for loc in Info::piece_disk_locs(info, piece) {
            self.done[loc.file] += (loc.end - loc.start) as u64;
            self.dirty.insert(loc.file);
        }
    }

    fn flush(&mut self) -> Vec<(usize, u64)> {
        let mut res = Vec::with_capacity(self.dirty.len());
        for idx in self.dirty.drain() {
            res.push((idx, self.done[idx]));
        }
        res
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
        import: bool,
    ) -> Torrent<T> {
        debug!("Creating {:?}", info);
        let peers = UHashMap::default();
        let pieces = Bitfield::new(u64::from(info.pieces()));
        let leechers = FHashSet::default();
        let mut status = Status {
            paused: !start,
            validating: None,
            error: None,
            state: if import {
                StatusState::Import
            } else {
                StatusState::Incomplete
            },
        };
        let priorities = Arc::new(vec![3; info.files.len()]);
        let info_idx = if info.complete() {
            None
        } else {
            status.state = StatusState::Magnet;
            Some(std::usize::MAX)
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
                        url: Arc::clone(&info.url_list[i][j]),
                    };
                    trackers.push_back(tracker);
                }
            }
        } else if let Some(ref announce) = info.announce {
            let tracker = Tracker {
                status: TrackerStatus::Updating,
                update: None,
                last_announce: Utc::now(),
                url: announce.clone(),
            };
            trackers.push_back(tracker);
        }

        let files = Files::new(&info, &pieces);

        let mut t = Torrent {
            id,
            info,
            path,
            peers,
            pieces,
            validating: FHashSet::default(),
            picker,
            priority: 3,
            priorities,
            uploaded: 0,
            downloaded: 0,
            wasted: 0,
            files,
            stat: stat::EMA::new(),
            cio,
            leechers,
            throttle,
            trackers,
            choker: choker::Choker::new(),
            dirty: true,
            status,
            info_bytes,
            info_idx,
            created: Utc::now(),
        };
        t.start(true);
        if import {
            t.cio.msg_disk(disk::Request::validate_piece(
                t.id,
                t.info.clone(),
                t.path.clone(),
                0,
            ));
            t.validating.insert(0);
        } else if CONFIG.disk.validate && t.info_idx.is_none() {
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
        let d = session::torrent::load(data)?;
        debug!("Torrent data deserialized!");
        let peers = UHashMap::default();
        let leechers = FHashSet::default();

        let info = Arc::new(Info {
            name: d.info.name,
            announce: d
                .info
                .announce
                .and_then(|u| Url::parse(&u).ok().map(Arc::new)),
            comment: d.info.comment,
            creator: d.info.creator,
            piece_len: d.info.piece_len,
            total_len: d.info.total_len,
            hashes: d.info.hashes,
            hash: d.info.hash,
            files: d
                .info
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
            Some(std::usize::MAX)
        };
        let info_bytes = if info_idx.is_none() {
            info.to_bencode().encode_to_buf()
        } else {
            vec![]
        };
        let pieces = Bitfield::from(&d.pieces.data, d.pieces.len);
        let picker = picker::Picker::new(&info, &pieces, &d.priorities);
        throttle.set_ul_rate(d.throttle_ul);
        throttle.set_dl_rate(d.throttle_dl);

        let mut trackers: VecDeque<_> = d
            .trackers
            .into_iter()
            .filter_map(|url| Url::parse(&url).ok())
            .map(|url| Tracker {
                status: TrackerStatus::Updating,
                update: None,
                last_announce: Utc::now(),
                url: Arc::new(url),
            })
            .collect();

        if trackers.is_empty() {
            if let Some(ref announce) = info.announce {
                let tracker = Tracker {
                    status: TrackerStatus::Updating,
                    update: None,
                    last_announce: Utc::now(),
                    url: announce.clone(),
                };
                trackers.push_back(tracker);
            }
        }

        let files = Files::new(&info, &pieces);

        let mut t = Torrent {
            id,
            info,
            peers,
            pieces,
            validating: FHashSet::default(),
            picker,
            uploaded: d.uploaded,
            downloaded: d.downloaded,
            wasted: 0,
            files,
            stat: stat::EMA::new(),
            priorities: Arc::new(d.priorities),
            priority: d.priority,
            cio,
            leechers,
            throttle,
            trackers,
            choker: choker::Choker::new(),
            dirty: false,
            status: Status {
                paused: d.status.paused,
                validating: None,
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
        t.start(false);
        if d.status.validating {
            t.validate();
        } else {
            t.announce_start();
        }
        Some(t)
    }

    pub fn serialize(&mut self) {
        let d = Session {
            info: session::torrent::current::Info {
                name: self.info.name.clone(),
                announce: self.info.announce.as_ref().map(|a| a.as_str().to_owned()),
                comment: self.info.comment.clone(),
                creator: self.info.creator.clone(),
                piece_len: self.info.piece_len,
                total_len: self.info.total_len,
                hashes: self.info.hashes.clone(),
                hash: self.info.hash,
                files: self
                    .info
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
            pieces: session::torrent::Bitfield {
                data: self.pieces.data(),
                len: self.pieces.len(),
            },
            uploaded: self.uploaded,
            downloaded: self.downloaded,
            status: session::torrent::current::Status {
                paused: self.status.paused,
                validating: self.status.validating.is_some(),
                error: self.status.error.clone(),
                state: match self.status.state {
                    StatusState::Magnet => session::torrent::current::StatusState::Magnet,
                    StatusState::Incomplete | StatusState::Import => {
                        session::torrent::current::StatusState::Incomplete
                    }
                    StatusState::Complete => session::torrent::current::StatusState::Complete,
                },
            },
            path: self.path.clone(),
            priorities: self.priorities.as_ref().clone(),
            priority: self.priority,
            created: self.created,
            throttle_ul: self.throttle.ul_rate(),
            throttle_dl: self.throttle.dl_rate(),
            trackers: self
                .trackers
                .iter()
                .map(|trk| trk.url.as_str().to_owned())
                .collect(),
        };
        let data = bincode::serialize(&d).expect("Serialization failed!");
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

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn priority(&self) -> u8 {
        self.priority
    }

    pub fn set_tracker_response(&mut self, url: &Url, resp: &tracker::Result<TrackerResponse>) {
        let mut time = Instant::now();
        let mut empty = false;
        match *resp {
            Ok(ref r) => {
                if let Some(tracker) = self.trackers.iter_mut().find(|t| &*t.url == url) {
                    debug!("Got valid response for {}", tracker.url);
                    time += Duration::from_secs(u64::from(r.interval));
                    tracker.status = TrackerStatus::Ok {
                        seeders: r.seeders,
                        leechers: r.leechers,
                        interval: r.interval,
                    };
                    tracker.update = Some(time);
                    tracker.last_announce = Utc::now();
                    if r.peers.is_empty() {
                        empty = true;
                    }
                }
            }
            Err(tracker::Error(tracker::ErrorKind::TrackerError(ref s), _)) => {
                if let Some(tracker) = self.trackers.iter_mut().find(|t| &*t.url == url) {
                    debug!("Got tracker level error for {}", tracker.url);
                    time += Duration::from_secs(300);
                    tracker.update = Some(time);
                    tracker.status = TrackerStatus::Failure(s.clone());
                    tracker.last_announce = Utc::now();
                }
            }
            Err(ref e) => {
                if let Some(tracker) = self.trackers.iter_mut().find(|t| &*t.url == url) {
                    error!("Failed to query tracker {}: {}", tracker.url, e);
                    // Wait 5 minutes before trying again
                    time += Duration::from_secs(300);
                    tracker.update = Some(time);
                    let reason = format!("Couldn't contact tracker: {}", e);
                    tracker.status = TrackerStatus::Failure(reason);
                    tracker.last_announce = Utc::now();
                }
            }
        }

        if (resp.is_err() || empty) && self.trackers.iter().any(|t| &*t.url == url) {
            if let Some(front) = self.trackers.pop_front() {
                self.trackers.push_back(front);
                self.try_update_tracker();
            }
        }
        self.update_rpc_tracker();
    }

    pub fn try_update_tracker(&mut self) {
        if self.status.stopped() {
            return;
        }
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

    pub fn update_tracker(&mut self) {
        if self.status.stopped() {
            return;
        }
        if let Some(req) = tracker::Request::interval(self) {
            self.cio.msg_trk(req);
        }
        self.dht_announce();
    }

    pub fn remove_peer(&mut self, rpc_id: &str) {
        let ih = &self.info.hash;
        let cio = &mut self.cio;
        if let Some((id, _)) = self
            .peers
            .iter()
            .find(|&(id, _)| util::peer_rpc_id(ih, *id as u64) == rpc_id)
        {
            cio.remove_peer(*id)
        }
    }

    pub fn add_tracker(&mut self, url: Url) -> String {
        let id = util::trk_rpc_id(&self.info.hash, url.as_str());
        self.trackers.push_front(Tracker {
            status: TrackerStatus::Updating,
            update: None,
            last_announce: Utc::now(),
            url: Arc::new(url),
        });
        {
            let trk = &self.trackers[0];
            let res = vec![resource::Resource::Tracker(resource::Tracker {
                id: id.clone(),
                torrent_id: self.rpc_id(),
                url: trk.url.as_ref().clone(),
                last_report: trk.last_announce,
                error: None,
                ..Default::default()
            })];
            self.cio.msg_rpc(rpc::CtlMessage::Extant(res));
        }
        self.announce_start();
        id
    }

    pub fn remove_tracker(&mut self, rpc_id: &str) {
        let ih = &self.info.hash;
        let mut res = None;
        for (idx, tracker) in self.trackers.iter().enumerate() {
            if util::trk_rpc_id(ih, tracker.url.as_str()) == rpc_id {
                res = Some(idx);
                self.cio
                    .msg_rpc(rpc::CtlMessage::Removed(vec![rpc_id.to_owned()]));
                break;
            }
        }

        if let Some(idx) = res {
            self.trackers.remove(idx);
        }
    }

    pub fn update_tracker_req(&mut self, rpc_id: &str) {
        if let Some(req) = self
            .trackers
            .iter()
            .find(|trk| util::trk_rpc_id(&self.info.hash, trk.url.as_str()) == rpc_id)
            .and_then(|trk| tracker::Request::custom(self, trk.url.clone()))
        {
            self.cio.msg_trk(req)
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
                    let p = Message::piece(context.idx, context.begin, context.length, data);
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
            disk::Response::PieceValidated { piece, valid, .. } => {
                self.validating.remove(&piece);
                if let StatusState::Import = self.status.state {
                    self.status.state = StatusState::Incomplete;
                    info!("Torrent imported!");
                    if valid {
                        for i in 0..self.info.pieces() {
                            self.pieces.set_bit(u64::from(i));
                        }
                        self.check_complete();
                    } else {
                        info!("Invalid torrent imported, redownloading!");
                    }
                    self.announce_start();
                    self.files.rebuild(&self.info, &self.pieces);
                    self.update_rpc_transfer();
                    return;
                }
                if valid {
                    self.pieces.set_bit(u64::from(piece));
                    // Tell all relevant peers we got the piece
                    let m = Message::Have(piece);
                    for pid in &self.leechers {
                        if let Some(peer) = self.peers.get_mut(pid) {
                            if !peer.pieces().has_bit(u64::from(piece)) {
                                peer.send_message(m.clone());
                            }
                        }
                    }
                    self.files.update(&self.info, piece);
                    self.check_complete();
                } else {
                    // TODO: trace down the bad peer and block it
                    debug!("Invalid piece downloaded!");
                    self.picker.invalidate_piece(piece);
                    if !self.stat.active() {
                        self.request_all();
                    }
                }
            }
            disk::Response::ValidationUpdate { percent, .. } => {
                self.status.validating = Some(percent);
                self.update_rpc_transfer();
            }
            disk::Response::ValidationComplete { mut invalid, .. } => {
                debug!("Validation completed!");
                self.status.validating = None;
                // Ignore invalid pieces which are
                // part of an invalid file(none of the disk locations
                // refer to files which aren't being downloaded(pri. 1)
                invalid.retain(|i| {
                    Info::piece_disk_locs(&self.info, *i).any(|loc| self.priorities[loc.file] != 0)
                });
                if invalid.is_empty() {
                    debug!("Torrent succesfully validated!");
                    if !self.complete() {
                        for i in 0..self.pieces.len() {
                            let complete = Info::piece_disk_locs(&self.info, i as u32)
                                .any(|loc| self.priorities[loc.file] != 0);
                            if complete {
                                self.pieces.set_bit(i);
                            }
                        }
                    }
                    self.set_finished();
                } else {
                    // If this is an initialization hash, start the torrent
                    // immediatly.
                    if self.pieces().iter().count() == 0 {
                        debug!("validation complete, starting torrent");
                        // If there was some partial completion,
                        // set the pieces appropriately, then reset the
                        // picker to use the new bitfield
                        if invalid.len() != self.pieces.len() as usize {
                            for i in 0..self.pieces.len() {
                                let complete = Info::piece_disk_locs(&self.info, i as u32)
                                    .any(|loc| self.priorities[loc.file] != 0);
                                if complete {
                                    self.pieces.set_bit(i);
                                }
                            }

                            for piece in invalid {
                                self.pieces.unset_bit(u64::from(piece));
                            }
                            let seq = self.picker.is_sequential();
                            self.change_picker(seq);
                        }
                        self.announce_start();
                    } else {
                        for piece in invalid {
                            self.picker.invalidate_piece(piece);
                            self.pieces.unset_bit(u64::from(piece));
                        }
                        self.request_all();
                    }
                    self.status.state = StatusState::Incomplete;
                }
                // update the RPC stats once done
                self.files.rebuild(&self.info, &self.pieces);
                self.update_rpc_transfer();
                self.rpc_update_pieces();
                self.announce_status();
            }
            disk::Response::Error { err, .. } => {
                error!("Disk error: {:?}", err);
                self.status.error = Some(format!("{}", err));
                self.announce_status();
                for piece in self.validating.drain() {
                    self.picker.invalidate_piece(piece);
                    self.pieces.unset_bit(u64::from(piece));
                }
            }
            disk::Response::FreeSpace(_) => unreachable!(),
        }
    }

    fn check_complete(&mut self) {
        let mut complete = true;
        for piece in 0..self.pieces.len() {
            let no_dl = Info::piece_disk_locs(&self.info, piece as u32)
                .all(|loc| self.priorities[loc.file] == 0);
            if self.pieces.has_bit(piece as u64) || no_dl {
                continue;
            } else {
                complete = false;
                break;
            }
        }

        if complete {
            if self.status.state != StatusState::Complete {
                self.status.state = StatusState::Complete;
                self.picker.done();
                self.set_finished();
                self.serialize();
            }
        } else if self.status.state == StatusState::Complete {
            self.status.state = StatusState::Incomplete;
            let seq = self.picker.is_sequential();
            self.picker = Picker::new(&self.info, &self.pieces, &self.priorities);
            self.change_picker(seq);
            self.announce_status();
            self.announce_start();
            self.request_all();
        }
    }
    /// Signal that we've downloaded and verified the torrent
    fn set_finished(&mut self) {
        info!("Torrent {} completed!", self.rpc_id());
        debug!("Wasted: {} MiB", (self.wasted * 16_384) / (1024 * 1024));
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
        {
            let seeders = self
                .peers
                .iter()
                .filter(|&(id, _)| !leechers.contains(id))
                .map(|(id, _)| *id);
            for seeder in seeders {
                self.cio.remove_peer(seeder);
            }
        }

        // Due to how we do validation updates, we should tell peers we now have every single piece
        for pid in leechers {
            if let Some(peer) = self.peers.get_mut(pid) {
                for i in 0..self.pieces.len() {
                    if !peer.pieces().has_bit(i) {
                        peer.send_message(Message::Have(i as u32));
                    }
                }
            }
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
                trace!(
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
                        b"ut_metadata".to_vec(),
                        bencode::BEncode::Int(i64::from(UT_META_ID)),
                    );
                    if !self.info.private {
                        m.insert(
                            b"ut_pex".to_vec(),
                            bencode::BEncode::Int(i64::from(UT_PEX_ID)),
                        );
                    }

                    ed.insert(b"m".to_vec(), bencode::BEncode::Dict(m));
                    ed.insert(
                        b"metadata_size".to_vec(),
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
                if self.pieces.usable(peer.pieces()) && self.status.validating.is_none() {
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
                if self.status.should_dl() && self.info.complete() {
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
                if self.pieces.has_bit(u64::from(index)) || self.validating.contains(&index) {
                    self.wasted += 1;
                    return Ok(());
                }

                // Even though we have the data, if we are stopped we shouldn't use the disk
                // regardless.
                if self.status.stopped() || self.status.completed() {
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

                let pr = {
                    let picker = &mut self.picker;
                    let peers = &mut self.peers;

                    picker.completed(Block::new(index, begin), |pid| {
                        if let Some(p) = peers.get_mut(&pid) {
                            p.send_message(Message::Cancel {
                                index,
                                begin,
                                length,
                            })
                        }
                    })
                };
                let piece_done = if let Ok(r) = pr {
                    r
                } else {
                    return Ok(());
                };

                self.dirty = true;
                self.write_piece(index, begin, data);

                self.downloaded += u64::from(length);
                self.stat.add_dl(u64::from(length));

                if piece_done {
                    self.cio.msg_disk(disk::Request::validate_piece(
                        self.id,
                        self.info.clone(),
                        self.path.clone(),
                        index,
                    ));
                    self.validating.insert(index);
                }

                if self.status.should_dl() {
                    Torrent::make_requests(peer, &mut self.picker, &self.info);
                }
            }
            Message::Request {
                index,
                begin,
                length,
            } => {
                if !self.pieces.has_bit(u64::from(index)) {
                    return Err(());
                }
                if length != self.info.block_len(index, begin) {
                    return Err(());
                }
                if !self.status.stopped() {
                    if let Some(buf) = Buffer::get() {
                        self.request_read(peer.id(), index, begin, buf);
                        return Ok(());
                    }
                }

                // TODO: add this to a queue to fulfill later
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
        }
        Ok(())
    }

    fn handle_ext(&mut self, id: u8, payload: Vec<u8>, peer: &mut Peer<T>) -> Result<(), ()> {
        if id == 0 {
            const MAX_INFO_BYTES: i64 = 100 * 1000 * 1000;
            let b = bencode::decode_buf(&payload).map_err(|_| ())?;
            let mut d = b.into_dict().ok_or(())?;
            let m = d
                .remove(b"m".as_ref())
                .and_then(|v| v.into_dict())
                .ok_or(())?;
            if m.contains_key(b"ut_metadata".as_ref()) {
                let size = d
                    .remove(b"metadata_size".as_ref())
                    .and_then(|v| v.into_int())
                    .ok_or(())?;
                if let Some(std::usize::MAX) = self.info_idx {
                    if size % 16_384 == 0 {
                        self.info_idx = Some(size as usize / 16_384 - 1);
                    } else {
                        self.info_idx = Some(size as usize / 16_384);
                    }
                    if size > MAX_INFO_BYTES {
                        debug!("UT metadata too large, {} MBs", size / (1000 * 1000));
                        return Err(());
                    }
                    self.info_bytes.resize(size as usize, 0u8);
                }
                if !self.info.complete() {
                    // Request the first index chunk to see if they have it
                    let mut respb = BTreeMap::new();
                    respb.insert(b"msg_type".to_vec(), bencode::BEncode::Int(0));
                    respb.insert(b"piece".to_vec(), bencode::BEncode::Int(0));
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
                return Ok(());
            };
            let buf = bencode::decode_buf_first(&payload).map_err(|_| ())?;
            let mut dict = buf.into_dict().ok_or(())?;
            let msg = dict
                .remove(b"msg_type".as_ref())
                .and_then(|v| v.into_int())
                .ok_or(())?;
            let piece_len = dict
                .remove(b"piece".as_ref())
                .and_then(|v| v.into_int())
                .ok_or(())? as usize;
            if piece_len * 16_384 >= self.info_bytes.len() {
                return Err(());
            }
            // Our metadata request strategy is as follows: after requesting the first
            // index chunk, we attempt to request every single subsequent chunk from
            // a peer which responds succesfully. This is slightly wasteful, but
            // simplifies logic (since we don't have to do "index piece picking").
            match msg {
                0 => {
                    let mut respb = BTreeMap::new();
                    if self.info_idx.is_none() {
                        respb.insert(b"msg_type".to_vec(), bencode::BEncode::Int(1));
                        respb.insert(b"piece".to_vec(), bencode::BEncode::Int(piece_len as i64));
                        let size = if self.info_bytes.len() / 16_384 == piece_len {
                            self.info_bytes.len() % 16_384
                        } else {
                            16_384
                        };
                        let total_size = self.info_bytes.len() as i64;
                        respb.insert(b"total_size".to_vec(), bencode::BEncode::Int(total_size));
                        let mut payload = bencode::BEncode::Dict(respb).encode_to_buf();
                        let s = piece_len * 16_384;
                        payload.extend_from_slice(&self.info_bytes[s..s + size]);
                        peer.send_message(Message::Extension {
                            id: utm_id,
                            payload,
                        });
                    } else {
                        respb.insert(b"msg_type".to_vec(), bencode::BEncode::Int(2));
                        respb.insert(b"piece".to_vec(), bencode::BEncode::Int(piece_len as i64));
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
                        if payload.len() - data_idx > self.info_bytes.len() - piece_len * 16_384 {
                            debug!(
                                "Metadata bounds invalid, goes to: {}, ibl: {}",
                                payload.len() - data_idx,
                                self.info_bytes.len() - piece_len * 16_384,
                            );
                            return Err(());
                        }
                        let size = if piece_len == idx {
                            self.info_bytes.len() - piece_len * 16_384
                        } else {
                            16_384
                        };
                        (&mut self.info_bytes[piece_len * 16_384..piece_len * 16_384 + size])
                            .copy_from_slice(&payload[data_idx..]);
                        if piece_len == idx {
                            let mut b = BTreeMap::new();
                            let bni = bencode::decode_buf(&self.info_bytes).map_err(|_| ())?;
                            b.insert(
                                b"announce".to_vec(),
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
                            b.insert(b"info".to_vec(), bni);
                            let ni =
                                Info::from_bencode(bencode::BEncode::Dict(b)).map_err(|_| ())?;
                            if ni.hash == self.info.hash {
                                debug!("Magnet file acquired succesfully!");
                                self.info_idx = None;
                                self.info = Arc::new(ni);
                                self.magnet_complete();
                            } else {
                                return Err(());
                            }
                        } else if piece_len == 0 {
                            for i in 1..=idx {
                                let mut respb = BTreeMap::new();
                                respb.insert(b"msg_type".to_vec(), bencode::BEncode::Int(0));
                                respb.insert(b"piece".to_vec(), bencode::BEncode::Int(i as i64));
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
        } else if id == UT_PEX_ID {
            const PEX_SEED: u8 = 0x02;
            const PEX_OUTGOING: u8 = 0x10;
            if peer.exts().ut_pex.is_none() {
                return Ok(());
            }
            if self.info.private {
                return Err(());
            }
            let b = bencode::decode_buf(&payload).map_err(|_| ())?;
            let mut d = b.into_dict().ok_or(())?;
            let mut peers = vec![];
            let flags = d
                .remove(b"added.f".as_ref())
                .and_then(bencode::BEncode::into_bytes)
                .unwrap_or_else(|| vec![0; 50]);
            if let Some(bencode::BEncode::String(ref data)) = d.remove(b"added".as_ref()) {
                for (p, flag) in data.chunks(6).zip(flags) {
                    if (flag & PEX_SEED != 0) && self.complete() {
                        continue;
                    }
                    if flag & PEX_OUTGOING == 0 {
                        continue;
                    }

                    let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
                    let socket = SocketAddrV4::new(ip, BigEndian::read_u16(&p[4..]));
                    peers.push(SocketAddr::V4(socket));
                }
            }
            if !peers.is_empty() {
                self.cio
                    .propagate(cio::Event::Tracker(Ok(tracker::Response::PEX {
                        tid: self.id,
                        peers,
                    })));
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

        match u.strategy {
            Some(resource::Strategy::Rarest) => self.change_picker(false),
            Some(resource::Strategy::Sequential) => self.change_picker(true),
            None => {}
        }

        if let Some(user_data) = u.user_data {
            let id = self.rpc_id();
            self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                resource::SResourceUpdate::UserData {
                    id,
                    kind: resource::ResourceKind::Torrent,
                    user_data,
                },
            ]));
        }
    }

    pub fn rpc_update_file(&mut self, id: String, priority: u8) {
        for (i, f) in self.info.files.iter().enumerate() {
            let fid =
                util::file_rpc_id(&self.info.hash, f.path.as_path().to_string_lossy().as_ref());
            if fid == id {
                Arc::make_mut(&mut self.priorities)[i] = priority;
            }
        }

        self.picker.set_priorities(&self.priorities, &self.info);
        self.clear_piece_cache();

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

    pub fn rpc_update_pieces(&mut self) {
        let id = self.rpc_id();
        let piece_field = self.pieces.b64();
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            resource::SResourceUpdate::TorrentPieces {
                id,
                kind: resource::ResourceKind::Torrent,
                piece_field,
            },
        ]));
    }

    fn start(&mut self, serialize: bool) {
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
        if serialize {
            self.serialize();
        }
    }

    fn announce_start(&mut self) {
        if self.status.stopped() {
            return;
        }
        if let Some(req) = tracker::Request::started(self) {
            self.cio.msg_trk(req);
            self.dump_torrent_file();
        }
        self.dht_announce();
    }

    fn dht_announce(&mut self) {
        if self.status.stopped() {
            return;
        }
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

    fn dump_torrent_file(&mut self) {
        let data = self.info.to_torrent_bencode().encode_to_buf();
        let mut path = PathBuf::from(&CONFIG.disk.session);
        path.push(&util::hash_to_id(&self.info.hash));
        path.set_extension("torrent");
        self.cio.msg_disk(disk::Request::WriteFile { data, path });
    }

    fn magnet_complete(&mut self) {
        self.status.state = StatusState::Incomplete;
        self.announce_status();
        self.pieces = Bitfield::new(u64::from(self.info.pieces()));
        self.priorities = Arc::new(vec![3; self.info.files.len()]);
        for peer in self.peers.values_mut() {
            if peer.magnet_complete(&self.info).is_err() {
                self.cio.remove_peer(peer.id());
            }
        }

        let resources = self.rpc_rel_info();
        self.cio.msg_rpc(rpc::CtlMessage::Extant(resources));
        let update = self.rpc_info();
        self.cio
            .msg_rpc(rpc::CtlMessage::Update(vec![SResourceUpdate::Resource(
                Cow::Owned(update),
            )]));
        self.serialize();

        let seq = self.picker.is_sequential();
        self.picker = Picker::new(&self.info, &self.pieces, &self.priorities);
        self.change_picker(seq);
        self.files = Files::new(&self.info, &self.pieces);
        self.validate();
        self.dump_torrent_file();
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
            strategy: if self.sequential() {
                resource::Strategy::Sequential
            } else {
                resource::Strategy::Rarest
            },
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
            piece_field: self.pieces.b64(),
            private: self.info.private,
            creator: self.info.creator.clone(),
            comment: self.info.comment.clone(),
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
        let mut seen_urls = FHashSet::default();
        self.trackers
            .iter()
            .filter_map(|trk| {
                if seen_urls.contains(trk.url.as_str()) {
                    return None;
                }
                seen_urls.insert(trk.url.as_str());
                Some(resource::Resource::Tracker(resource::Tracker {
                    id: util::trk_rpc_id(&self.info.hash, trk.url.as_str()),
                    torrent_id: self.rpc_id(),
                    url: trk.url.as_ref().clone(),
                    last_report: trk.last_announce,
                    error: None,
                    ..Default::default()
                }))
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
        let mut seen_urls = FHashSet::default();
        for (_, tracker) in self.trackers.iter().enumerate() {
            if seen_urls.contains(tracker.url.as_str()) {
                continue;
            }
            seen_urls.insert(tracker.url.as_str());
            r.push(util::trk_rpc_id(&self.info.hash, tracker.url.as_str()));
        }
        self.cio.msg_rpc(rpc::CtlMessage::Removed(r));
    }

    fn error(&self) -> Option<String> {
        self.status.error.clone()
    }

    fn sequential(&self) -> bool {
        self.picker.is_sequential()
    }

    fn progress(&self) -> f32 {
        if self.status.magnet() {
            return 0.0;
        }
        if let Some(amnt) = self.status.validating {
            amnt
        } else {
            self.pieces.iter().count() as f32 / self.info.pieces() as f32
        }
    }

    fn availability(&self) -> f32 {
        if self.leechers.len() != self.peers.len() {
            return 1.0;
        }
        if self.status.magnet() {
            return 0.0;
        }
        let mut peers_have = FHashSet::default();
        for peer in self.peers.values() {
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
    fn write_piece(&mut self, index: u32, begin: u32, data: Buffer) {
        let locs = Info::block_disk_locs_pri(&self.info, &self.priorities, index, begin);
        self.cio
            .msg_disk(disk::Request::write(self.id, data, locs, self.path.clone()));
    }

    /// Issues a read request of the given torrent
    fn request_read(&mut self, id: usize, index: u32, begin: u32, data: Buffer) {
        let locs = Info::block_disk_locs(&self.info, index, begin);
        let len = self.info.block_len(index, begin);
        let ctx = disk::Ctx::new(id, self.id, index, begin, len);
        self.cio
            .msg_disk(disk::Request::read(ctx, data, locs, self.path.clone()));
    }

    fn make_requests_pid(&mut self, pid: usize) {
        if self.status.should_dl() {
            let peer = self
                .peers
                .get_mut(&pid)
                .expect("Expected peer id not present");
            Torrent::make_requests(peer, &mut self.picker, &self.info);
        }
    }

    fn make_requests(peer: &mut Peer<T>, picker: &mut Picker, info: &Info) {
        if let Some(m) = peer.queue_reqs() {
            for _ in 0..(m) {
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
        } else {
            peer.interested();
        }
    }

    pub fn add_peer(&mut self, conn: PeerConn) -> Option<usize> {
        if self.peers.len() >= MAX_PEERS {
            return None;
        }
        if self.peers.values().any(|p| p.addr() == conn.sock().addr()) {
            return None;
        }
        if let Ok(pid) = self.cio.add_peer(conn) {
            if let Ok(p) = Peer::new(pid, self, None, None) {
                if self.info_idx.is_none() {
                    self.picker.add_peer(&p);
                }
                self.peers.insert(pid, p);
                return Some(pid);
            }
        }
        None
    }

    pub fn add_inc_peer(&mut self, pid: usize, id: [u8; 20], rsv: [u8; 8]) -> Option<usize> {
        if let Some(addr) = self.cio.get_peer(pid, |pconn| pconn.sock().addr()) {
            if self.peers.values().any(|p| p.addr() == addr) {
                return None;
            }
        }
        if let Ok(p) = Peer::new(pid, self, Some(id), Some(rsv)) {
            debug!("{:?}: Adding peer {:?}!", self.rpc_id(), pid);
            if self.info_idx.is_none() {
                self.picker.add_peer(&p);
            }
            self.peers.insert(pid, p);
            return Some(pid);
        }
        None
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
        let updates = self
            .trackers
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
                    last_report: tracker.last_announce,
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
            id,
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

        for (idx, done) in self.files.flush() {
            let id = util::file_rpc_id(
                &self.info.hash,
                self.info.files[idx].path.to_string_lossy().as_ref(),
            );
            updates.push(SResourceUpdate::FileProgress {
                id,
                kind: resource::ResourceKind::File,
                progress: (done as f32 / self.info.files[idx].length as f32),
            });
        }
        self.announce_status();
        self.cio.msg_rpc(rpc::CtlMessage::Update(updates));
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
            self.dht_announce();
        }
    }

    pub fn validate(&mut self) {
        self.cio.msg_disk(disk::Request::validate(
            self.id,
            self.info.clone(),
            self.path.clone(),
        ));
        self.status.validating = Some(0.0);
        self.announce_status();
    }

    pub fn num_peers(&self) -> usize {
        self.peers.len()
    }

    pub fn peers(&self) -> &UHashMap<Peer<T>> {
        &self.peers
    }

    pub fn update_pex(&mut self, added: &[SocketAddr], removed: &[SocketAddr]) {
        let mut a = vec![];
        let mut a6 = vec![];
        let mut r = vec![];
        let mut r6 = vec![];
        for addr in added {
            match &addr {
                SocketAddr::V4(addr) => {
                    a.extend(&addr.ip().octets());
                }
                SocketAddr::V6(addr) => {
                    a6.extend(&addr.ip().octets());
                }
            }
        }
        for addr in removed {
            match &addr {
                SocketAddr::V4(addr) => {
                    r.extend(&addr.ip().octets());
                }
                SocketAddr::V6(addr) => {
                    r6.extend(&addr.ip().octets());
                }
            }
        }
        let mut dict = BTreeMap::new();
        dict.insert(b"added".to_vec(), BEncode::String(a));
        dict.insert(b"added6".to_vec(), BEncode::String(a6));
        dict.insert(b"removed".to_vec(), BEncode::String(r));
        dict.insert(b"removed6".to_vec(), BEncode::String(r6));
        let payload = BEncode::Dict(dict).encode_to_buf();

        for peer in self.peers.values_mut() {
            if let Some(id) = peer.exts().ut_pex {
                peer.send_message(Message::Extension {
                    id,
                    payload: payload.clone(),
                });
            }
        }
    }

    pub fn rank_peers(&mut self) {
        let mut pids = self.pids();
        pids.sort_by_key(|pid| self.peers.get(pid).unwrap().get_tx_rates().1);
        pids.reverse();
        if !pids.is_empty() {
            debug!(
                "Lowest ranked peer dl {}",
                self.peers[&pids[0]].get_tx_rates().1
            );
        }
        for (rank, pid) in pids.into_iter().enumerate() {
            self.peers.get_mut(&pid).unwrap().rank = rank;
        }
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
        let prev_seq = self.picker.is_sequential();
        self.picker.change_picker(sequential);
        for peer in self.peers.values() {
            self.picker.add_peer(peer);
        }
        self.picker.set_priorities(&self.priorities, &self.info);
        let id = self.rpc_id();
        let sequential = self.picker.is_sequential();
        self.clear_piece_cache();
        if prev_seq != sequential {
            self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                SResourceUpdate::TorrentPicker {
                    id,
                    kind: resource::ResourceKind::Torrent,
                    strategy: if sequential {
                        resource::Strategy::Sequential
                    } else {
                        resource::Strategy::Rarest
                    },
                },
            ]));
        }
    }

    fn clear_piece_cache(&mut self) {
        for peer in self.peers.values_mut() {
            peer.piece_cache().clear();
        }
    }
}

impl<T: cio::CIO> fmt::Debug for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Torrent {{ info: {:?} }}", self.info)
    }
}

impl<T: cio::CIO> fmt::Display for Torrent<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Torrent {}", util::hash_to_id(&self.info.hash[..]))
    }
}

impl<T: cio::CIO> Drop for Torrent<T> {
    fn drop(&mut self) {
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
