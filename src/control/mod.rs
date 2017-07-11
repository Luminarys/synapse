use std::{thread, fmt, fs, io, time};
use slog::Logger;
use std::io::{Read};
use {rpc, tracker, disk, listener, DISK, RPC, CONFIG, TC, TRACKER, LISTENER};
use util::io_err;
use amy::{self, Poller, Registrar};
use torrent::{self, peer, Torrent};
use std::collections::HashMap;
use bencode::BEncode;
use std::sync::{Arc, Mutex};
use throttle::Throttler;

mod job;

/// Throttler max token amount
const THROT_TOKS: usize = 2 * 1024 * 1024;
/// Tracker update job interval
const TRK_JOB_SECS: u64 = 60;
/// Unchoke rotation job interval
const UNCHK_JOB_SECS: u64 = 30;
/// Session serialization job interval
const SES_JOB_SECS: u64 = 10;
/// Bad peer reap interval
const REAP_JOB_SECS: u64 = 2;
/// Interval to requery all jobs and execute if needed
const JOB_INT_MS: usize = 1000;
/// Interval to poll for events
const POLL_INT_MS: usize = 3;

pub struct Control {
    trk_rx: amy::Receiver<tracker::Response>,
    disk_rx: amy::Receiver<disk::Response>,
    ctrl_rx: amy::Receiver<Request>,
    throttler: Throttler,
    reg: Arc<Registrar>,
    poll: Poller,
    tid_cnt: usize,
    job_timer: usize,
    jobs: job::JobManager,
    torrents: HashMap<usize, Torrent>,
    peers: HashMap<usize, usize>,
    hash_idx: HashMap<[u8; 20], usize>,
    l: Logger,
}

pub struct Handle {
    pub trk_tx: Mutex<amy::Sender<tracker::Response>>,
    pub disk_tx: Mutex<amy::Sender<disk::Response>>,
    pub ctrl_tx: Mutex<amy::Sender<Request>>,
}

unsafe impl Sync for Handle {}

pub enum Request {
    AddTorrent(BEncode),
    AddPeer(peer::PeerConn, [u8; 20]),
    RPC(rpc::Request),
    Shutdown,
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Request")
    }
}

impl Control {
    pub fn new(poll: Poller,
               trk_rx: amy::Receiver<tracker::Response>,
               disk_rx: amy::Receiver<disk::Response>,
               ctrl_rx: amy::Receiver<Request>,
               l: Logger) -> Control {
        let torrents = HashMap::new();
        let peers = HashMap::new();
        let hash_idx = HashMap::new();
        let reg = Arc::new(poll.get_registrar().unwrap());
        // Every minute check to update trackers;
        let mut jobs = job::JobManager::new();
        jobs.add_job(job::TrackerUpdate, time::Duration::from_secs(TRK_JOB_SECS));
        jobs.add_job(job::UnchokeUpdate, time::Duration::from_secs(UNCHK_JOB_SECS));
        jobs.add_job(job::SessionUpdate, time::Duration::from_secs(SES_JOB_SECS));
        jobs.add_job(job::ReapPeers, time::Duration::from_secs(REAP_JOB_SECS));
        let job_timer = reg.set_interval(JOB_INT_MS).unwrap();
        // 5 MiB max bucket
        let throttler = Throttler::new(0, 0, THROT_TOKS, &reg);
        Control { trk_rx, disk_rx, ctrl_rx, poll, torrents, peers,
        hash_idx, reg, tid_cnt: 0, throttler, jobs, job_timer, l }
    }

    pub fn run(&mut self) {
        if self.deserialize().is_err() {
            warn!(self.l, "Session deserialization failed!");
        }
        debug!(self.l, "Initialized!");
        loop {
            for event in self.poll.wait(POLL_INT_MS).unwrap() {
                if self.handle_event(event) {
                    self.serialize();
                    return;
                }
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
        let r = self.reg.clone();
        let throttle = self.throttler.get_throttle(tid);
        let log = self.l.new(o!("torrent" => tid));
        if let Ok(t) = Torrent::deserialize(tid, &data, throttle, r, log) {
            trace!(self.l, "Succesfully parsed torrent file {:?}", dir.path());
            self.hash_idx.insert(t.info().hash, tid);
            self.tid_cnt += 1;
            self.torrents.insert(tid, t);
        } else {
            return io_err("Torrent data invalid!");
        }
        Ok(())
    }

    fn handle_event(&mut self, not: amy::Notification) -> bool {
        match not.id {
            id if id == self.trk_rx.get_id() => self.handle_trk_ev(),
            id if id == self.disk_rx.get_id() => self.handle_disk_ev(),
            id if id == self.ctrl_rx.get_id() => return self.handle_ctrl_ev(),
            id if id == self.throttler.id() => self.throttler.update(),
            id if id == self.throttler.fid() => self.flush_blocked_peers(),
            id if id == self.job_timer => self.update_jobs(),
            _ => self.handle_peer_ev(not),
        }
        false
    }

    fn handle_trk_ev(&mut self) {
        debug!(self.l, "Handling tracker response");
        while let Ok((id, resp)) = self.trk_rx.try_recv() {
            {
                let torrent = self.torrents.get_mut(&id).unwrap();
                torrent.set_tracker_response(&resp);
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
    }

    fn update_jobs(&mut self) {
        debug!(self.l, "Handling job timer");
        self.jobs.update(&mut self.torrents);
    }

    fn handle_disk_ev(&mut self) {
        while let Ok(resp) = self.disk_rx.try_recv() {
            trace!(self.l, "Got disk response {:?}!", resp);
            let torrent = &mut self.torrents.get_mut(&resp.tid()).unwrap();
            torrent.handle_disk_resp(resp);
        }
    }

    fn handle_ctrl_ev(&mut self) -> bool {
        loop {
            match self.ctrl_rx.try_recv() {
                Ok(Request::AddTorrent(b)) => {
                    if let Ok(i) = torrent::Info::from_bencode(b) {
                        self.add_torrent(i);
                    }
                }
                Ok(Request::AddPeer(p, hash)) => {
                    trace!(self.l, "Adding peer for torrent with hash {:?}!", hash);
                    if let Some(tid) = self.hash_idx.get(&hash).cloned() {
                        self.add_peer(tid, p);
                    } else {
                        warn!(self.l, "Couldn't add peer, torrent with hash {:?} doesn't exist", hash);
                    }
                }
                Ok(Request::RPC(r)) => {
                    self.handle_rpc(r);
                }
                Ok(Request::Shutdown) => {
                    return true;
                }
                Err(_) => { break; }
            }
        }
        false
    }

    fn handle_peer_ev(&mut self, not: amy::Notification) {
        let pid = not.id;
        if not.event.readable() {
            self.peer_readable(pid);
        }
        if not.event.writable() {
            self.peer_writable(pid);
        }
    }

    fn flush_blocked_peers(&mut self) {
        trace!(self.l, "Flushing blocked peer!");
        for pid in self.throttler.flush_dl() {
            trace!(self.l, "Flushing blocked peer!");
            self.peer_readable(pid);
        }
        for pid in self.throttler.flush_ul() {
            self.peer_writable(pid);
        }
    }

    fn peer_readable(&mut self, pid: usize) {
        let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
        trace!(self.l, "Peer {:?} readable", pid);
        torrent.peer_readable(pid);
    }

    fn peer_writable(&mut self, pid: usize) {
        let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
        trace!(self.l, "Peer {:?} writable", pid);
        torrent.peer_writable(pid);
    }

    fn add_torrent(&mut self, info: torrent::Info) {
        debug!(self.l, "Adding {:?}!", info);
        if self.hash_idx.contains_key(&info.hash) {
            warn!(self.l, "Torrent already exists!");
            return;
        }
        let tid = self.tid_cnt;
        let r = self.reg.clone();
        let throttle = self.throttler.get_throttle(tid);
        let log = self.l.new(o!("torrent" => tid));
        let t = Torrent::new(tid, info, throttle, r, log);
        self.hash_idx.insert(t.info().hash, tid);
        self.tid_cnt += 1;
        self.torrents.insert(tid, t);
    }

    fn handle_rpc(&mut self, req: rpc::Request) {
        match req {
            rpc::Request::ListTorrents => {
                let mut resp = Vec::new();
                for (id, _) in self.torrents.iter() {
                    resp.push(*id);
                }
                RPC.tx.send(rpc::Response::Torrents(resp)).unwrap();
            }
            rpc::Request::TorrentInfo(i) => {
                if let Some(torrent) = self.torrents.get(&i) {
                    RPC.tx.send(rpc::Response::TorrentInfo(torrent.rpc_info())).unwrap();
                } else {
                    RPC.tx.send(rpc::Response::Err("Torrent ID not found!".to_owned())).unwrap();
                }
            }
            rpc::Request::AddTorrent(data) => {
                match torrent::Info::from_bencode(data) {
                    Ok(i) => {
                        self.add_torrent(i);
                        RPC.tx.send(rpc::Response::Ack).unwrap();
                    }
                    Err(e) => {
                        RPC.tx.send(rpc::Response::Err(e.to_owned())).unwrap();
                    }
                }
            }
            rpc::Request::PauseTorrent(id) => {
                if let Some(t) = self.torrents.get_mut(&id) {
                    t.pause();
                    RPC.tx.send(rpc::Response::Ack).unwrap();
                } else {
                    RPC.tx.send(rpc::Response::Err("Torrent not found!".to_owned())).unwrap();
                }
            }
            rpc::Request::ResumeTorrent(id) => {
                if let Some(t) = self.torrents.get_mut(&id) {
                    t.resume();
                    RPC.tx.send(rpc::Response::Ack).unwrap();
                } else {
                    RPC.tx.send(rpc::Response::Err("Torrent not found!".to_owned())).unwrap();
                }
            }
            rpc::Request::RemoveTorrent(id) => {
                if let Some(t) = self.torrents.remove(&id) {
                    self.hash_idx.remove(&t.info().hash);
                    t.delete();
                    RPC.tx.send(rpc::Response::Ack).unwrap();
                } else {
                    RPC.tx.send(rpc::Response::Err("Torrent not found!".to_owned())).unwrap();
                }
            }
            rpc::Request::ThrottleUpload(amnt) => {
                self.throttler.set_ul_rate(amnt);
                RPC.tx.send(rpc::Response::Ack).unwrap();
            }
            rpc::Request::ThrottleDownload(amnt) => {
                self.throttler.set_dl_rate(amnt);
                RPC.tx.send(rpc::Response::Ack).unwrap();
            }
            rpc::Request::Shutdown => { unimplemented!(); }
        }
    }

    fn add_peer(&mut self, id: usize, peer: peer::PeerConn) {
        trace!(self.l, "Adding peer to torrent {:?}!", id);
        let torrent = self.torrents.get_mut(&id).unwrap();
        if let Some(pid) = torrent.add_peer(peer) {
            self.peers.insert(pid, id);
        }
    }
}

pub fn start(l: Logger) -> Handle {
    debug!(l, "Initializing!");
    let poll = amy::Poller::new().unwrap();
    let mut reg = poll.get_registrar().unwrap();
    let (trk_tx, trk_rx) = reg.channel().unwrap();
    let (disk_tx, disk_rx) = reg.channel().unwrap();
    let (ctrl_tx, ctrl_rx) = reg.channel().unwrap();
    thread::spawn(move || {
        {
            Control::new(poll, trk_rx, disk_rx, ctrl_rx, l.clone()).run();
            use std::sync::atomic;
            TC.fetch_sub(1, atomic::Ordering::SeqCst);
        }
        debug!(l, "Triggering thread shutdown sequence!");
        DISK.tx.send(disk::Request::shutdown()).unwrap();
        RPC.rtx.send(rpc::Request::Shutdown).unwrap();
        TRACKER.tx.send(tracker::Request::Shutdown).unwrap();
        LISTENER.tx.send(listener::Request::Shutdown).unwrap();
        debug!(l, "Shutdown!");
    });
    Handle { trk_tx: Mutex::new(trk_tx), disk_tx: Mutex::new(disk_tx), ctrl_tx: Mutex::new(ctrl_tx) }
}
