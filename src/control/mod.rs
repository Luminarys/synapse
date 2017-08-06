use std::{fs, io, time};
use slog::Logger;
use std::io::Read;
use std::sync::atomic;
use {rpc, tracker, disk, listener, CONFIG, SHUTDOWN};
use util::{io_err, io_err_val};
use torrent::{self, peer, Torrent};
use std::collections::HashMap;
use throttle::Throttler;

pub mod cio;
pub mod acio;
mod job;

/// Tracker update job interval
const TRK_JOB_SECS: u64 = 60;
/// Unchoke rotation job interval
const UNCHK_JOB_SECS: u64 = 30;
/// Session serialization job interval
const SES_JOB_SECS: u64 = 10;
/// Interval to requery all jobs and execute if needed
const JOB_INT_MS: usize = 1000;

pub struct Control<T: cio::CIO> {
    throttler: Throttler,
    cio: T,
    tid_cnt: usize,
    job_timer: usize,
    jobs: job::JobManager<T>,
    torrents: HashMap<usize, Torrent<T>>,
    peers: HashMap<usize, usize>,
    hash_idx: HashMap<[u8; 20], usize>,
    l: Logger,
}

impl<T: cio::CIO> Control<T> {
    pub fn new(mut cio: T,
               throttler: Throttler,
               l: Logger) -> io::Result<Control<T>> {
        let torrents = HashMap::new();
        let peers = HashMap::new();
        let hash_idx = HashMap::new();
        // Every minute check to update trackers;
        let mut jobs = job::JobManager::new();
        jobs.add_job(job::TrackerUpdate, time::Duration::from_secs(TRK_JOB_SECS));
        jobs.add_job(job::UnchokeUpdate, time::Duration::from_secs(UNCHK_JOB_SECS));
        jobs.add_job(job::SessionUpdate, time::Duration::from_secs(SES_JOB_SECS));
        let job_timer = cio.set_timer(JOB_INT_MS).map_err(|_| io_err_val("timer failure!"))?;
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
            l
        })
    }

    pub fn run(&mut self) {
        if self.deserialize().is_err() {
            warn!(self.l, "Session deserialization failed!");
        }
        debug!(self.l, "Initialized!");
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
            return Ok(())
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
                    self.throttler.update();
                } else if t == self.throttler.fid() {
                    self.flush_blocked_peers();
                } else if t == self.job_timer {
                    self.update_jobs();
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
            self.add_peer(tid, msg.peer);
        } else {
            warn!(self.l, "Couldn't add peer, torrent with hash {:?} doesn't exist", msg.hash);
        }
    }

    fn handle_peer_ev(&mut self, peer: cio::PID, ev: cio::Result<torrent::Message>) {
        let ref mut p = self.peers;
        let ref mut t = self.torrents;

        p.get(&peer).cloned()
            .and_then(|id| t.get_mut(&id))
            .map(|torrent| {
                if torrent.peer_ev(peer, ev).is_err() {
                    p.remove(&peer);
                }
            });
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

    fn send_rpc_msg(&mut self, resp: rpc::Response) {
        self.cio.msg_rpc(rpc::CMessage::Response(resp));
    }

    fn handle_rpc_ev(&mut self, req: rpc::Request) -> bool {
        debug!(self.l, "Handling rpc reqest!");
        /*
        match req {
            rpc::Request::ListTorrents => {
                let mut resp = Vec::new();
                for (id, _) in self.torrents.iter() {
                    resp.push(*id);
                }
                self.send_rpc_msg(rpc::Response::Torrents(resp));
            }
            rpc::Request::TorrentInfo(i) => {
                let resp = if let Some(torrent) = self.torrents.get(&i) {
                    rpc::Response::TorrentInfo(torrent.rpc_info())
                } else {
                    rpc::Response::Err("Torrent ID not found!".to_owned())
                };
                self.send_rpc_msg(resp);
            }
            rpc::Request::AddTorrent(data) => {
                let resp = match torrent::Info::from_bencode(data) {
                    Ok(i) => {
                        self.add_torrent(i);
                        rpc::Response::Ack
                    }
                    Err(e) => {
                        rpc::Response::Err(e.to_owned())
                    }
                };
                self.send_rpc_msg(resp);
            }
            rpc::Request::PauseTorrent(id) => {
                let resp = if let Some(t) = self.torrents.get_mut(&id) {
                    t.pause();
                    rpc::Response::Ack
                } else {
                    rpc::Response::Err("Torrent not found!".to_owned())
                };
                self.send_rpc_msg(resp);
            }
            rpc::Request::ResumeTorrent(id) => {
                let resp = if let Some(t) = self.torrents.get_mut(&id) {
                    t.resume();
                    rpc::Response::Ack
                } else {
                    rpc::Response::Err("Torrent not found!".to_owned())
                };
                self.send_rpc_msg(resp);
            }
            rpc::Request::RemoveTorrent(id) => {
                let resp = if let Some(mut t) = self.torrents.remove(&id) {
                    self.hash_idx.remove(&t.info().hash);
                    t.delete();
                    rpc::Response::Ack
                } else {
                    rpc::Response::Err("Torrent not found!".to_owned())
                };
                self.send_rpc_msg(resp);
            }
            rpc::Request::ThrottleUpload(amnt) => {
                self.throttler.set_ul_rate(amnt);
                self.send_rpc_msg(rpc::Response::Ack);
            }
            rpc::Request::ThrottleDownload(amnt) => {
                self.throttler.set_dl_rate(amnt);
                self.send_rpc_msg(rpc::Response::Ack);
            }
            rpc::Request::Shutdown => { return true; }
        }
        */
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
}

impl<T: cio::CIO> Drop for Control<T> {
    fn drop(&mut self) {
        debug!(self.l, "Triggering thread shutdown sequence!");
        self.cio.msg_disk(disk::Request::shutdown());
        self.cio.msg_rpc(rpc::CMessage::Shutdown);
        self.cio.msg_trk(tracker::Request::Shutdown);
        self.cio.msg_listener(listener::Request::Shutdown);
    }
}
