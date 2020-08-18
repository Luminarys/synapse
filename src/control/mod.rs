use std::io::Read;
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic;
use std::{fs, io, mem, process, time};

use chrono::Utc;

use crate::throttle::Throttler;
use crate::torrent::{self, peer, Torrent};
use crate::util::{
    self, hash_to_id, id_to_hash, io_err, io_err_val, random_string, FHashSet, MHashMap, UHashMap,
    UHashSet,
};
use crate::{disk, rpc, stat, tracker, CONFIG, DL_TOKEN, SHUTDOWN};

pub mod acio;
pub mod cio;
mod job;

/// Tracker update job interval
const TRK_JOB_SECS: u64 = 60;
/// Unchoke rotation job interval
const UNCHK_JOB_SECS: u64 = 15;
/// Session serialization job interval
const SES_JOB_SECS: u64 = 60;
/// Interval to update RPC of transfer stats
const TX_JOB_MS: u64 = 500;
/// Interval to check space on disk
const SPACE_JOB_SECS: u64 = 10;
/// Interval to send PEX updates
const PEX_JOB_SECS: u64 = 60 * 5;
/// Interval to enqueue new torrents
const ENQUEUE_JOB_SECS: u64 = 5;

/// Interval to requery all jobs and execute if needed
const JOB_INT_MS: usize = 500;

pub struct Control<T: cio::CIO> {
    throttler: Throttler,
    cio: T,
    tid_cnt: usize,
    job_timer: usize,
    stat: stat::EMA,
    jobs: JobManager<T>,
    torrents: UHashMap<Torrent<T>>,
    queue: Queue,
    peers: UHashMap<usize>,
    incoming: UHashSet,
    hash_idx: MHashMap<[u8; 20], usize>,
    data: ServerData,
    db: amy::Sender<disk::Request>,
}

#[derive(Serialize, Deserialize, Default)]
struct ServerData {
    id: String,
    ul: u64,
    dl: u64,
    #[serde(skip)]
    session_ul: u64,
    #[serde(skip)]
    session_dl: u64,
    #[serde(skip)]
    free_space: u64,
    throttle_ul: Option<i64>,
    throttle_dl: Option<i64>,
}

struct Queue {
    active_dl: FHashSet<usize>,
    inactive_dl: [FHashSet<usize>; 6],
}

pub trait CJob<T: cio::CIO> {
    fn update(&mut self, control: &mut Control<T>);
}

struct JobManager<T: cio::CIO> {
    jobs: Vec<JobData<Box<dyn job::Job<T>>>>,
    cjobs: Vec<JobData<Box<dyn CJob<T>>>>,
}

struct JobData<T> {
    job: T,
    last_updated: time::Instant,
    interval: time::Duration,
}

impl<T: cio::CIO> Control<T> {
    pub fn new(
        mut cio: T,
        throttler: Throttler,
        db: amy::Sender<disk::Request>,
    ) -> io::Result<Control<T>> {
        let torrents = UHashMap::default();
        let peers = UHashMap::default();
        let incoming = UHashSet::default();
        let hash_idx = MHashMap::default();
        let mut jobs = JobManager::new();

        jobs.add_job(job::TrackerUpdate, time::Duration::from_secs(TRK_JOB_SECS));
        jobs.add_job(
            job::UnchokeUpdate,
            time::Duration::from_secs(UNCHK_JOB_SECS),
        );
        jobs.add_job(job::SessionUpdate, time::Duration::from_secs(SES_JOB_SECS));
        jobs.add_job(
            job::TorrentTxUpdate::new(),
            time::Duration::from_millis(TX_JOB_MS),
        );
        jobs.add_job(
            job::PEXUpdate::new(),
            time::Duration::from_secs(PEX_JOB_SECS),
        );

        jobs.add_cjob(SpaceUpdate, time::Duration::from_secs(SPACE_JOB_SECS));
        jobs.add_cjob(EnqueueUpdate, time::Duration::from_secs(ENQUEUE_JOB_SECS));
        jobs.add_cjob(SerializeUpdate, time::Duration::from_secs(SES_JOB_SECS));
        let job_timer = cio
            .set_timer(JOB_INT_MS)
            .map_err(|_| io_err_val("timer failure!"))?;
        Ok(Control {
            throttler,
            cio,
            tid_cnt: 0,
            job_timer,
            jobs,
            torrents,
            peers,
            incoming,
            hash_idx,
            stat: stat::EMA::new(),
            data: Default::default(),
            db,
            queue: Queue::new(),
        })
    }

    pub fn run(&mut self) {
        if self.deserialize().is_err() {
            error!("Session deserialization failed!");
        }
        debug!("Initialized!");
        self.send_rpc_info();
        let mut events = Vec::with_capacity(20);
        'outer: loop {
            if let Err(e) = self.cio.poll(&mut events) {
                error!("{}", e);
                break;
            }
            for event in events.drain(..) {
                if self.handle_event(event) {
                    break 'outer;
                }
            }
            if SHUTDOWN.load(atomic::Ordering::SeqCst) {
                break;
            }
        }
        self.serialize();
    }

    fn serialize(&mut self) {
        let sd = &CONFIG.disk.session;
        debug!("Serializing server data!");
        let mut path = PathBuf::from(sd);
        path.push("syn_data");
        match bincode::serialize(&self.data) {
            Ok(data) => {
                self.db.send(disk::Request::WriteFile { path, data }).ok();
            }
            Err(_) => {
                error!("Failed to serialize server data");
            }
        }
        debug!("Serializing torrents!");
        for torrent in self.torrents.values_mut() {
            torrent.serialize();
        }
    }

    fn deserialize(&mut self) -> io::Result<()> {
        let sd = &CONFIG.disk.session;
        debug!("Deserializing server data!");
        let mut pb = PathBuf::from(sd);
        pb.push("syn_data");
        if let Ok(Ok(data)) = fs::File::open(pb).map(|mut f| bincode::deserialize_from(&mut f)) {
            self.data = data;
            self.throttler.set_ul_rate(self.data.throttle_ul);
            self.throttler.set_dl_rate(self.data.throttle_dl);
        } else {
            error!("No server data found, regenerating!");
            self.data = ServerData::new();
        }

        debug!("Deserializing torrents!");
        for entry in fs::read_dir(sd)? {
            if self.deserialize_torrent(entry).is_err() {
                error!(
                    "Please ensure that session data is not corrupted and not past version {}",
                    env!("CARGO_PKG_VERSION")
                );
                process::exit(1);
            }
        }
        Ok(())
    }

    fn deserialize_torrent(&mut self, entry: io::Result<fs::DirEntry>) -> io::Result<()> {
        let dir = entry?;
        // TODO: We probably should improve this heuristic with and not rely
        // on directory entries, but this is good enough for now.
        if dir.file_name().len() != 40 {
            return Ok(());
        }
        trace!("Attempting to deserialize file {:?}", dir);
        let mut f = fs::File::open(dir.path())?;
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        trace!("Succesfully read file");

        let tid = self.tid_cnt;
        let throttle = self.throttler.get_throttle(tid);
        if let Some(t) = Torrent::deserialize(tid, &data, throttle, self.cio.new_handle()) {
            trace!("Succesfully parsed torrent file {:?}", dir.path());
            self.hash_idx.insert(t.info().hash, tid);
            self.tid_cnt += 1;
            if t.status().leeching() {
                self.queue.add(tid, t.priority());
            }
            self.torrents.insert(tid, t);
        } else {
            error!("Failed to deserialize torrent {:?}", dir.file_name());
            return io_err("Torrent data invalid!");
        }
        Ok(())
    }

    fn handle_event(&mut self, event: cio::Event) -> bool {
        match event {
            cio::Event::Tracker(Ok(e)) => {
                self.handle_trk_ev(e);
            }
            cio::Event::Tracker(Err(e)) => {
                error!("tracker error: {}", e);
                trace!("tracker error bt: {:?}", e.backtrace());
            }
            cio::Event::Disk(Ok(e)) => {
                self.handle_disk_ev(e);
            }
            cio::Event::Disk(Err(e)) => {
                error!("disk error: {}", e);
                trace!("disk error: {:?}", e.backtrace());
            }
            cio::Event::RPC(Ok(e)) => {
                return self.handle_rpc_ev(e);
            }
            cio::Event::RPC(Err(e)) => {
                error!("rpc error: {}", e);
                trace!("rpc error: {:?}", e.backtrace());
            }
            cio::Event::Incoming(conn) => {
                self.handle_incoming_conn(conn);
            }
            cio::Event::Timer(t) => {
                if t == self.throttler.id() {
                    let (ul, dl) = self.throttler.update();
                    self.data.ul += ul;
                    self.data.dl += dl;
                    self.data.session_ul += ul;
                    self.data.session_dl += dl;
                    self.stat.add_ul(ul);
                    self.stat.add_dl(dl);
                } else if t == self.throttler.fid() {
                    self.flush_blocked_peers();
                } else if t == self.job_timer {
                    self.update_jobs();
                    self.update_rpc_tx();
                } else {
                    error!("unknown timer id {} reported", t);
                }
            }
            cio::Event::Peer { peer, event } => {
                self.handle_peer_ev(peer, event);
            }
        }
        false
    }

    fn handle_trk_ev(&mut self, tr: tracker::Response) {
        let (id, peers) = match tr {
            tracker::Response::Tracker { tid, url, resp } => {
                debug!("Handling tracker response for {:?}", url);
                if let Some(torrent) = self.torrents.get_mut(&tid) {
                    torrent.set_tracker_response(url.as_ref(), &resp);
                    if let Ok(r) = resp {
                        (tid, r.peers)
                    } else {
                        return;
                    }
                } else {
                    return;
                }
            }
            tracker::Response::DHT { tid, peers } | tracker::Response::PEX { tid, peers } => {
                (tid, peers)
            }
        };
        for ip in &peers {
            trace!("Adding peer({:?})!", ip);
            if let Ok(peer) = peer::PeerConn::new_outgoing(ip) {
                trace!("Added peer({:?})!", ip);
                self.add_peer(id, peer);
            }
        }
    }

    fn update_jobs(&mut self) {
        trace!("Handling job timer");
        let mut jobs = mem::replace(&mut self.jobs, JobManager::new());
        jobs.update(self);
        self.jobs = jobs;
    }

    fn handle_disk_ev(&mut self, resp: disk::Response) {
        trace!("Got disk response {:?}!", resp);
        if let disk::Response::FreeSpace(space) = resp {
            if space / 1_000_000 != self.data.free_space / 1_000_000 {
                self.data.free_space = space;
                self.update_rpc_space();
            }
        } else if let Some(torrent) = self.torrents.get_mut(&resp.tid()) {
            torrent.handle_disk_resp(resp);
        }
    }

    fn handle_incoming_conn(&mut self, conn: TcpStream) {
        match peer::PeerConn::new_incoming(conn) {
            Ok(pconn) => match self.cio.add_peer(pconn) {
                Ok(pid) => {
                    self.incoming.insert(pid);
                }
                Err(e) => {
                    error!("Failed to add peer connection: {:?}", e);
                }
            },
            Err(e) => {
                error!("Failed to create peer connection: {:?}", e);
            }
        }
    }

    fn inc_handshake(
        &mut self,
        pid: cio::PID,
        ev: cio::Result<torrent::Message>,
    ) -> Result<(), ()> {
        match ev {
            Ok(msg) => match msg {
                torrent::Message::Handshake { hash, id, rsv } => {
                    debug!("Adding peer for torrent with hash {:?}!", hash_to_id(&hash));
                    if let Some(tid) = self.hash_idx.get(&hash).cloned() {
                        return self.add_inc_peer(tid, pid, id, rsv);
                    } else {
                        error!(
                            "Couldn't add peer, torrent {} doesn't exist",
                            hash_to_id(&hash)
                        );
                    }
                }
                // The Reader is instantiated in State::Handshake, so
                // the first message must be a handshake.
                _ => unreachable!(),
            },
            Err(_) => return Err(()),
        }
        Err(())
    }

    fn handle_peer_ev(&mut self, pid: cio::PID, ev: cio::Result<torrent::Message>) {
        let p = &mut self.peers;

        if let Some(&tid) = p.get(&pid) {
            let t = &mut self.torrents;
            if let Some(torrent) = t.get_mut(&tid) {
                if torrent.peer_ev(pid, ev).is_err() {
                    p.remove(&pid);
                    torrent.update_rpc_peers();
                }
            }
        } else if self.incoming.remove(&pid) {
            if self.inc_handshake(pid, ev).is_err() {
                self.cio.remove_peer(pid);
            }
        }
    }

    fn flush_blocked_peers(&mut self) {
        trace!("Flushing blocked peers!");
        self.cio.flush_peers(self.throttler.flush_dl());
        self.cio.flush_peers(self.throttler.flush_ul());
    }

    fn add_torrent(
        &mut self,
        info: torrent::Info,
        path: Option<String>,
        start: bool,
        import: bool,
        client: usize,
        serial: u64,
    ) {
        debug!("Adding {:?}, start: {}!", info, start);
        let id = hash_to_id(&info.hash);
        if self.hash_idx.contains_key(&info.hash) {
            debug!("Tried to add torrent that already exists!");
            self.cio.msg_rpc(rpc::CtlMessage::Error {
                client,
                serial,
                reason: format!("Torrent {} already exists", id),
            });
            return;
        }
        let tid = self.tid_cnt;
        let throttle = self.throttler.get_throttle(tid);
        let t = Torrent::new(
            tid,
            path,
            info,
            throttle,
            self.cio.new_handle(),
            start,
            import,
        );
        self.hash_idx.insert(t.info().hash, tid);
        self.tid_cnt += 1;
        self.queue.add(tid, t.priority());
        self.torrents.insert(tid, t);
        self.cio
            .msg_rpc(rpc::CtlMessage::Uploaded { id, client, serial })
    }

    fn handle_rpc_ev(&mut self, req: rpc::Message) -> bool {
        debug!("Handling rpc reqest!");
        match req {
            rpc::Message::UpdateTorrent(u) => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let res = id_to_hash(&u.id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i));
                if let Some(t) = res {
                    let old_pri = t.priority();
                    t.rpc_update(u);
                    let new_pri = t.priority();
                    self.queue.modify_pri(t.id(), new_pri, old_pri);
                }
            }
            rpc::Message::Torrent {
                info,
                path,
                start,
                import,
                client,
                serial,
            } => self.add_torrent(info, path, start, import, client, serial),
            rpc::Message::UpdateFile {
                id,
                torrent_id,
                priority,
            } => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let res = id_to_hash(&torrent_id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i));
                if let Some(t) = res {
                    t.rpc_update_file(id, priority);
                }
            }
            rpc::Message::AddPeer {
                id,
                client,
                serial,
                peer,
            } => {
                let res = id_to_hash(&id)
                    .and_then(|d| self.hash_idx.get(d.as_ref()))
                    .cloned();
                let pres = peer::PeerConn::new_outgoing(&peer);
                if let Some(tid) = res {
                    if let Ok(pc) = pres {
                        if let Some(id) = self.add_peer_rpc(tid, pc) {
                            self.cio
                                .msg_rpc(rpc::CtlMessage::Pending { id, client, serial });
                        } else {
                            self.cio.msg_rpc(rpc::CtlMessage::Error {
                                client,
                                serial,
                                reason: format!("Could not add peer {}", peer),
                            });
                        }
                    } else {
                        self.cio.msg_rpc(rpc::CtlMessage::Error {
                            client,
                            serial,
                            reason: format!("Could not create peer {}", peer),
                        });
                    }
                } else {
                    self.cio.msg_rpc(rpc::CtlMessage::Error {
                        client,
                        serial,
                        reason: format!("torrent {} does not exist", id),
                    });
                }
            }
            rpc::Message::AddTracker {
                id,
                client,
                serial,
                tracker,
            } => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let cio = &mut self.cio;
                let reason = format!("Could not add tracker {}", tracker);
                id_to_hash(&id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                    .map(|t| t.add_tracker(tracker))
                    .map(|id| cio.msg_rpc(rpc::CtlMessage::Uploaded { id, client, serial }))
                    .unwrap_or_else(|| {
                        cio.msg_rpc(rpc::CtlMessage::Error {
                            reason,
                            client,
                            serial,
                        })
                    });
            }
            rpc::Message::UpdateServer {
                id,
                throttle_up,
                throttle_down,
            } => {
                let tu = throttle_up.unwrap_or_else(|| self.throttler.ul_rate());
                let td = throttle_down.unwrap_or_else(|| self.throttler.dl_rate());
                self.throttler.set_ul_rate(tu);
                self.throttler.set_dl_rate(td);
                self.data.throttle_ul = tu;
                self.data.throttle_dl = td;
                self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                    rpc::resource::SResourceUpdate::Throttle {
                        id,
                        kind: rpc::resource::ResourceKind::Server,
                        throttle_up: tu,
                        throttle_down: td,
                    },
                ]));
            }
            rpc::Message::RemoveTorrent {
                id,
                client,
                serial,
                artifacts,
            } => {
                let hash_idx = &mut self.hash_idx;
                let torrents = &mut self.torrents;
                let cio = &mut self.cio;
                let reason = format!("Torrent {} does not exist", id);
                id_to_hash(&id)
                    .and_then(|d| hash_idx.remove(d.as_ref()))
                    .and_then(|i| torrents.remove(&i))
                    .map(|mut t| t.delete(artifacts))
                    .map(|_| cio.msg_rpc(rpc::CtlMessage::ClientRemoved { id, client, serial }))
                    .unwrap_or_else(|| {
                        cio.msg_rpc(rpc::CtlMessage::Error {
                            client,
                            serial,
                            reason,
                        })
                    });
            }
            rpc::Message::Pause(id) => {
                let hash_idx = &mut self.hash_idx;
                let torrents = &mut self.torrents;
                if let Some(t) = id_to_hash(&id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                {
                    t.pause()
                }
            }
            rpc::Message::Resume(id) => {
                let hash_idx = &mut self.hash_idx;
                let torrents = &mut self.torrents;
                if let Some(t) = id_to_hash(&id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                {
                    t.resume();
                }
            }
            rpc::Message::Validate(ids) => {
                let hash_idx = &mut self.hash_idx;
                let torrents = &mut self.torrents;
                for id in ids {
                    if let Some(t) = id_to_hash(&id)
                        .and_then(|d| hash_idx.get(d.as_ref()))
                        .and_then(|i| torrents.get_mut(i))
                    {
                        t.validate();
                    }
                }
            }
            rpc::Message::RemovePeer {
                id,
                torrent_id,
                client,
                serial,
            } => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let cio = &mut self.cio;
                let reason = "Torrent or peer does not exist!".to_string();
                id_to_hash(&torrent_id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                    .map(|t| t.remove_peer(&id))
                    .map(|_| cio.msg_rpc(rpc::CtlMessage::ClientRemoved { id, client, serial }))
                    .unwrap_or_else(|| {
                        cio.msg_rpc(rpc::CtlMessage::Error {
                            client,
                            serial,
                            reason,
                        })
                    });
            }
            rpc::Message::RemoveTracker {
                id,
                torrent_id,
                client,
                serial,
            } => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let cio = &mut self.cio;
                let reason = "Torrent or tracker does not exist!".to_string();
                id_to_hash(&torrent_id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                    .map(|t| t.remove_tracker(&id))
                    .map(|_| cio.msg_rpc(rpc::CtlMessage::ClientRemoved { id, client, serial }))
                    .unwrap_or_else(|| {
                        cio.msg_rpc(rpc::CtlMessage::Error {
                            client,
                            serial,
                            reason,
                        })
                    });
            }
            rpc::Message::UpdateTracker { id, torrent_id } => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                if let Some(t) = id_to_hash(&torrent_id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i))
                {
                    t.update_tracker_req(&id);
                }
            }
            rpc::Message::PurgeDNS => {
                self.cio.msg_trk(tracker::Request::PurgeDNS);
            }
        }
        false
    }

    fn add_peer_rpc(&mut self, id: usize, peer: peer::PeerConn) -> Option<String> {
        trace!("Adding peer to torrent {:?}!", id);
        if let Some(torrent) = self.torrents.get_mut(&id) {
            if let Some(pid) = torrent.add_peer(peer) {
                self.peers.insert(pid, id);
                return Some(util::peer_rpc_id(&torrent.info().hash, pid as u64));
            }
        }
        None
    }

    fn add_peer(&mut self, id: usize, peer: peer::PeerConn) {
        trace!("Adding peer to torrent {:?}!", id);
        if let Some(torrent) = self.torrents.get_mut(&id) {
            if !self.queue.active_dl.contains(&id) && !torrent.status().completed() {
                self.queue.add(id, torrent.priority());
                return;
            }
            if let Some(pid) = torrent.add_peer(peer) {
                self.peers.insert(pid, id);
            }
        }
    }

    fn add_inc_peer(
        &mut self,
        id: usize,
        pid: usize,
        cid: [u8; 20],
        rsv: [u8; 8],
    ) -> Result<(), ()> {
        trace!("Adding peer to torrent {:?}!", id);
        if let Some(torrent) = self.torrents.get_mut(&id) {
            if !self.queue.active_dl.contains(&id) && !torrent.status().completed() {
                self.queue.add(id, torrent.priority());
                return Err(());
            }
            if let Some(pid) = torrent.add_inc_peer(pid, cid, rsv) {
                self.peers.insert(pid, id);
                return Ok(());
            }
        }
        Err(())
    }

    fn update_rpc_space(&mut self) {
        self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
            rpc::resource::SResourceUpdate::ServerSpace {
                id: self.data.id.clone(),
                kind: rpc::resource::ResourceKind::Server,
                free_space: self.data.free_space,
            },
        ]));
    }

    fn update_rpc_tx(&mut self) {
        self.stat.tick();
        if self.stat.active() {
            let (ul, dl) = (self.stat.avg_ul(), self.stat.avg_dl());
            self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                rpc::resource::SResourceUpdate::ServerTransfer {
                    id: self.data.id.clone(),
                    kind: rpc::resource::ResourceKind::Server,
                    rate_up: ul,
                    rate_down: dl,
                    transferred_up: self.data.ul,
                    transferred_down: self.data.dl,
                    ses_transferred_up: self.data.session_ul,
                    ses_transferred_down: self.data.session_dl,
                },
            ]));
        }
    }

    fn send_rpc_info(&mut self) {
        let res = rpc::resource::Resource::Server(rpc::resource::Server {
            id: self.data.id.clone(),
            rate_up: 0,
            rate_down: 0,
            throttle_up: self.throttler.ul_rate(),
            throttle_down: self.throttler.dl_rate(),
            transferred_up: self.data.ul,
            transferred_down: self.data.dl,
            ses_transferred_up: self.data.session_ul,
            ses_transferred_down: self.data.session_dl,
            free_space: self.data.free_space,
            started: Utc::now(),
            download_token: DL_TOKEN.clone(),
            ..Default::default()
        });
        self.cio.msg_rpc(rpc::CtlMessage::Extant(vec![res]));
    }
}

impl<T: cio::CIO> Drop for Control<T> {
    fn drop(&mut self) {
        debug!("Triggering thread shutdown sequence!");
        self.torrents.drain().last();
        self.cio.msg_rpc(rpc::CtlMessage::Shutdown);
        self.cio.msg_trk(tracker::Request::Shutdown);
        self.cio.msg_disk(disk::Request::shutdown());
    }
}

impl ServerData {
    pub fn new() -> ServerData {
        ServerData {
            id: env!("CARGO_PKG_VERSION").to_owned() + "-" + &random_string(15),
            ul: 0,
            dl: 0,
            session_ul: 0,
            session_dl: 0,
            free_space: 0,
            throttle_ul: Some(-1),
            throttle_dl: Some(-1),
        }
    }
}

impl Queue {
    fn new() -> Queue {
        let inactive_dl = [
            FHashSet::default(),
            FHashSet::default(),
            FHashSet::default(),
            FHashSet::default(),
            FHashSet::default(),
            FHashSet::default(),
        ];
        Queue {
            active_dl: FHashSet::default(),
            inactive_dl,
        }
    }

    fn dl_full(&self) -> bool {
        self.active_dl.len() == CONFIG.max_dl as usize
    }

    fn modify_pri(&mut self, id: usize, pri: u8, old_pri: u8) {
        let pri = pri as usize;
        let old_pri = old_pri as usize;
        self.inactive_dl[old_pri].remove(&id);
        self.inactive_dl[pri].insert(id);
    }

    fn add(&mut self, id: usize, pri: u8) {
        let pri = pri as usize;
        if self.dl_full() {
            self.inactive_dl[pri].insert(id);
        } else {
            self.active_dl.insert(id);
        }
    }

    fn enqueue<F: FnMut(usize)>(&mut self, mut f: F) {
        while !self.dl_full() && self.inactive_dl.iter().any(|q| !q.is_empty()) {
            for i in (0..self.inactive_dl.len()).rev() {
                if !self.inactive_dl[i].is_empty() {
                    let next = { *self.inactive_dl[i].iter().next().unwrap() };
                    self.inactive_dl[i].remove(&next);
                    self.active_dl.insert(next);
                    f(next);
                    break;
                }
            }
        }
    }
}

impl<T: cio::CIO> JobManager<T> {
    pub fn new() -> JobManager<T> {
        JobManager {
            jobs: Vec::with_capacity(0),
            cjobs: Vec::with_capacity(0),
        }
    }

    pub fn add_job<J: job::Job<T> + 'static>(&mut self, job: J, interval: time::Duration) {
        self.jobs.push(JobData {
            job: Box::new(job),
            interval,
            last_updated: time::Instant::now(),
        })
    }

    pub fn add_cjob<J: CJob<T> + 'static>(&mut self, job: J, interval: time::Duration) {
        self.cjobs.push(JobData {
            job: Box::new(job),
            interval,
            last_updated: time::Instant::now(),
        })
    }

    pub fn update(&mut self, control: &mut Control<T>) {
        for j in &mut self.jobs {
            if j.last_updated.elapsed() > j.interval {
                j.job.update(&mut control.torrents);
                j.last_updated = time::Instant::now();
            }
        }
        for j in &mut self.cjobs {
            if j.last_updated.elapsed() > j.interval {
                j.job.update(control);
                j.last_updated = time::Instant::now();
            }
        }
    }
}

pub struct SpaceUpdate;

impl<T: cio::CIO> CJob<T> for SpaceUpdate {
    fn update(&mut self, control: &mut Control<T>) {
        control.cio.msg_disk(disk::Request::FreeSpace);
    }
}

pub struct EnqueueUpdate;

impl<T: cio::CIO> CJob<T> for EnqueueUpdate {
    fn update(&mut self, control: &mut Control<T>) {
        let queue = &mut control.queue;
        let torrents = &mut control.torrents;

        queue.active_dl.retain(|tid| match torrents.get(tid) {
            Some(t) => t.status().should_dl(),
            None => false,
        });
        for q in &mut queue.inactive_dl {
            q.retain(|tid| torrents.contains_key(tid));
        }
        queue.enqueue(|tid| torrents.get_mut(&tid).unwrap().update_tracker());
    }
}

pub struct SerializeUpdate;

impl<T: cio::CIO> CJob<T> for SerializeUpdate {
    fn update(&mut self, control: &mut Control<T>) {
        control.serialize();
    }
}
