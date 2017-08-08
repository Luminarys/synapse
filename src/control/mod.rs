use std::{fs, io, time};
use std::io::Read;
use std::sync::atomic;
use std::collections::HashMap;

use slog::Logger;
use chrono::Utc;

use {rpc, tracker, disk, listener, CONFIG, SHUTDOWN, PEER_ID};
use util::{io_err, io_err_val, id_to_hash, hash_to_id};
use torrent::{self, peer, Torrent};
use throttle::Throttler;

pub mod cio;
pub mod acio;
mod job;

/// Tracker update job interval
const TRK_JOB_SECS: u64 = 60;
/// Unchoke rotation job interval
const UNCHK_JOB_SECS: u64 = 15;
/// Session serialization job interval
const SES_JOB_SECS: u64 = 10;
/// Interval to update RPC of transfer stats
const TX_JOB_MS: u64 = 333;

/// Interval to requery all jobs and execute if needed
const JOB_INT_MS: usize = 333;

pub struct Control<T: cio::CIO> {
    throttler: Throttler,
    cio: T,
    tid_cnt: usize,
    job_timer: usize,
    tx_rates: Option<(u64, u64)>,
    jobs: job::JobManager<T>,
    torrents: HashMap<usize, Torrent<T>>,
    peers: HashMap<usize, usize>,
    hash_idx: HashMap<[u8; 20], usize>,
    l: Logger,
}

impl<T: cio::CIO> Control<T> {
    pub fn new(mut cio: T, throttler: Throttler, l: Logger) -> io::Result<Control<T>> {
        let torrents = HashMap::new();
        let peers = HashMap::new();
        let hash_idx = HashMap::new();
        // Every minute check to update trackers;
        let mut jobs = job::JobManager::new();
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
        let job_timer = cio.set_timer(JOB_INT_MS).map_err(
            |_| io_err_val("timer failure!"),
        )?;
        // 5 MiB max bucket
        Ok(Control {
            throttler,
            cio,
            tid_cnt: 0,
            job_timer,
            jobs,
            torrents,
            peers,
            hash_idx,
            tx_rates: None,
            l,
        })
    }

    pub fn run(&mut self) {
        if self.deserialize().is_err() {
            warn!(self.l, "Session deserialization failed!");
        }
        debug!(self.l, "Initialized!");
        self.send_rpc_info();
        let mut events = Vec::with_capacity(20);
        loop {
            self.cio.poll(&mut events);
            for event in events.drain(..) {
                if self.handle_event(event) {
                    self.serialize();
                    return;
                }
            }
            if SHUTDOWN.load(atomic::Ordering::SeqCst) == true {
                break;
            }
        }
    }

    fn serialize(&mut self) {
        debug!(self.l, "Serializing torrents!");
        for (_, torrent) in self.torrents.iter_mut() {
            torrent.serialize();
        }
    }

    fn deserialize(&mut self) -> io::Result<()> {
        debug!(self.l, "Deserializing torrents!");
        let sd = &CONFIG.session;
        for entry in fs::read_dir(sd)? {
            if let Err(e) = self.deserialize_torrent(entry) {
                warn!(self.l, "Failed to deserialize torrent file: {:?}!", e);
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
        trace!(self.l, "Attempting to deserialize file {:?}", dir);
        let mut f = fs::File::open(dir.path())?;
        let mut data = Vec::new();
        f.read_to_end(&mut data)?;
        trace!(self.l, "Succesfully read file");

        let tid = self.tid_cnt;
        let throttle = self.throttler.get_throttle(tid);
        let log = self.l.new(o!("torrent" => tid));
        if let Ok(t) = Torrent::deserialize(tid, &data, throttle, self.cio.new_handle(), log) {
            trace!(self.l, "Succesfully parsed torrent file {:?}", dir.path());
            self.hash_idx.insert(t.info().hash, tid);
            self.tid_cnt += 1;
            self.torrents.insert(tid, t);
        } else {
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
                error!(self.l, "tracker error: {:?}", e.backtrace());
            }
            cio::Event::Disk(Ok(e)) => {
                self.handle_disk_ev(e);
            }
            cio::Event::Disk(Err(e)) => {
                error!(self.l, "disk error: {:?}", e.backtrace());
            }
            cio::Event::RPC(Ok(e)) => {
                return self.handle_rpc_ev(e);
            }
            cio::Event::RPC(Err(e)) => {
                error!(self.l, "rpc error: {:?}", e.backtrace());
            }
            cio::Event::Listener(Ok(e)) => {
                self.handle_lst_ev(e);
            }
            cio::Event::Listener(Err(e)) => {
                error!(self.l, "listener error: {:?}", e.backtrace());
            }
            cio::Event::Timer(t) => {
                if t == self.throttler.id() {
                    if let Some(p) = self.throttler.update() {
                        self.tx_rates = Some(p);
                    }
                } else if t == self.throttler.fid() {
                    self.flush_blocked_peers();
                } else if t == self.job_timer {
                    self.update_jobs();
                    self.update_rpc_tx();
                } else {
                    error!(self.l, "unknown timer id {} reported", t);
                }
            }
            cio::Event::Peer { peer, event } => {
                self.handle_peer_ev(peer, event);
            }
        }
        false
    }

    fn handle_trk_ev(&mut self, tr: tracker::Response) {
        debug!(self.l, "Handling tracker response");
        let id = tr.0;
        let resp = tr.1;
        {
            if let Some(torrent) = self.torrents.get_mut(&id) {
                torrent.set_tracker_response(&resp);
            } else {
                return;
            }
        }
        trace!(self.l, "Adding peers!");
        if let Ok(r) = resp {
            for ip in r.peers.iter() {
                trace!(self.l, "Adding peer({:?})!", ip);
                if let Ok(peer) = peer::PeerConn::new_outgoing(ip) {
                    trace!(self.l, "Added peer({:?})!", ip);
                    self.add_peer(id, peer);
                }
            }
            if let Some(torrent) = self.torrents.get_mut(&id) {
                torrent.update_rpc_peers();
            }
        }
    }

    fn update_jobs(&mut self) {
        trace!(self.l, "Handling job timer");
        self.jobs.update(&mut self.torrents);
    }

    fn handle_disk_ev(&mut self, resp: disk::Response) {
        trace!(self.l, "Got disk response {:?}!", resp);
        if let Some(torrent) = self.torrents.get_mut(&resp.tid()) {
            torrent.handle_disk_resp(resp);
        }
    }

    fn handle_lst_ev(&mut self, msg: listener::Message) {
        debug!(self.l, "Adding peer for torrent with hash {:?}!", msg.hash);
        if let Some(tid) = self.hash_idx.get(&msg.hash).cloned() {
            self.add_inc_peer(tid, msg.peer, msg.id, msg.rsv);
        } else {
            warn!(
                self.l,
                "Couldn't add peer, torrent with hash {:?} doesn't exist",
                msg.hash
            );
        }
    }

    fn handle_peer_ev(&mut self, peer: cio::PID, ev: cio::Result<torrent::Message>) {
        let ref mut p = self.peers;
        let ref mut t = self.torrents;

        p.get(&peer).cloned().and_then(|id| t.get_mut(&id)).map(
            |torrent| if torrent.peer_ev(peer, ev).is_err() {
                p.remove(&peer);
            },
        );
    }

    fn flush_blocked_peers(&mut self) {
        trace!(self.l, "Flushing blocked peers!");
        self.cio.flush_peers(self.throttler.flush_dl());
        self.cio.flush_peers(self.throttler.flush_ul());
    }

    fn add_torrent(&mut self, info: torrent::Info) {
        debug!(self.l, "Adding {:?}!", info);
        if self.hash_idx.contains_key(&info.hash) {
            warn!(self.l, "Torrent already exists!");
            return;
        }
        let tid = self.tid_cnt;
        let throttle = self.throttler.get_throttle(tid);
        let log = self.l.new(o!("torrent" => tid));
        let t = Torrent::new(tid, info, throttle, self.cio.new_handle(), log);
        self.hash_idx.insert(t.info().hash, tid);
        self.tid_cnt += 1;
        self.torrents.insert(tid, t);
    }

    fn handle_rpc_ev(&mut self, req: rpc::Message) -> bool {
        debug!(self.l, "Handling rpc reqest!");
        match req {
            rpc::Message::UpdateTorrent(u) => {
                let hash_idx = &self.hash_idx;
                let torrents = &mut self.torrents;
                let res = id_to_hash(&u.id)
                    .and_then(|d| hash_idx.get(d.as_ref()))
                    .and_then(|i| torrents.get_mut(i));
                if let Some(t) = res {
                    t.rpc_update(u);
                }
            }
            rpc::Message::Torrent(i) => self.add_torrent(i),
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
            rpc::Message::UpdateServer {
                id,
                throttle_up,
                throttle_down,
            } => {
                let tu = throttle_up.unwrap_or(self.throttler.ul_rate() as u32);
                let td = throttle_down.unwrap_or(self.throttler.dl_rate() as u32);
                self.throttler.set_ul_rate(tu as usize);
                self.throttler.set_dl_rate(td as usize);
                self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                    rpc::resource::SResourceUpdate::ServerThrottle {
                        id,
                        throttle_up: tu,
                        throttle_down: td,
                    },
                ]));
            }
            _ => {}
        }
        false
    }

    fn add_peer(&mut self, id: usize, peer: peer::PeerConn) {
        trace!(self.l, "Adding peer to torrent {:?}!", id);
        if let Some(torrent) = self.torrents.get_mut(&id) {
            if let Some(pid) = torrent.add_peer(peer) {
                self.peers.insert(pid, id);
            }
        }
    }

    fn add_inc_peer(&mut self, id: usize, peer: peer::PeerConn, cid: [u8; 20], rsv: [u8; 8]) {
        trace!(self.l, "Adding peer to torrent {:?}!", id);
        if let Some(torrent) = self.torrents.get_mut(&id) {
            if let Some(pid) = torrent.add_inc_peer(peer, cid, rsv) {
                self.peers.insert(pid, id);
            }
        }
    }

    fn update_rpc_tx(&mut self) {
        if let Some((rate_up, rate_down)) = self.tx_rates {
            self.cio.msg_rpc(rpc::CtlMessage::Update(vec![
                rpc::resource::SResourceUpdate::ServerTransfer {
                    id: hash_to_id(&PEER_ID[..]),
                    rate_up,
                    rate_down,
                },
            ]));
            self.tx_rates = None;
        }
    }

    fn send_rpc_info(&mut self) {
        let res = rpc::resource::Resource::Server(rpc::resource::Server {
            id: hash_to_id(&PEER_ID[..]),
            rate_up: 0,
            rate_down: 0,
            throttle_up: 0,
            throttle_down: 0,
            started: Utc::now(),
        });
        self.cio.msg_rpc(rpc::CtlMessage::Extant(vec![res]));
    }
}

impl<T: cio::CIO> Drop for Control<T> {
    fn drop(&mut self) {
        debug!(self.l, "Triggering thread shutdown sequence!");
        self.cio.msg_disk(disk::Request::shutdown());
        self.cio.msg_rpc(rpc::CtlMessage::Shutdown);
        self.cio.msg_trk(tracker::Request::Shutdown);
        self.cio.msg_listener(listener::Request::Shutdown);
    }
}
