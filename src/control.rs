use std::{thread, fmt};
use {rpc, tracker, disk, TRACKER, RPC};
use amy::{self, Poller, Registrar};
use torrent::{self, Torrent, Peer};
use std::collections::HashMap;
use bencode::BEncode;
use std::sync::{Arc, Mutex};
use throttle::Throttler;

pub struct Control {
    trk_rx: amy::Receiver<tracker::Response>,
    disk_rx: amy::Receiver<disk::Response>,
    ctrl_rx: amy::Receiver<Request>,
    throttler: Throttler,
    reg: Arc<Registrar>,
    poll: Poller,
    tid_cnt: usize,
    torrents: HashMap<usize, Torrent>,
    peers: HashMap<usize, usize>,
    hash_idx: HashMap<[u8; 20], usize>,
}

pub struct Handle {
    pub trk_tx: Mutex<amy::Sender<tracker::Response>>,
    pub disk_tx: Mutex<amy::Sender<disk::Response>>,
    pub ctrl_tx: Mutex<amy::Sender<Request>>,
}

unsafe impl Sync for Handle {}

pub enum Request {
    AddTorrent(BEncode),
    AddPeer(Peer, [u8; 20]),
    RPC(rpc::Request),
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
               ctrl_rx: amy::Receiver<Request>) -> Control {
        let torrents = HashMap::new();
        let peers = HashMap::new();
        let hash_idx = HashMap::new();
        let reg = Arc::new(poll.get_registrar().unwrap());
        // 5 MiB max bucket
        let throttler = Throttler::new(3000, 10 * 1024 * 1024, &reg);
        Control { trk_rx, disk_rx, ctrl_rx, poll, torrents, peers, hash_idx, reg, tid_cnt: 0, throttler }
    }

    pub fn run(&mut self) {
        loop {
            for event in self.poll.wait(5).unwrap() {
                self.handle_event(event);
            }
        }
    }

    fn handle_event(&mut self, not: amy::Notification) {
        match not.id {
            id if id == self.trk_rx.get_id() => self.handle_trk_ev(),
            id if id == self.disk_rx.get_id() => self.handle_disk_ev(),
            id if id == self.ctrl_rx.get_id() => self.handle_ctrl_ev(),
            id if id == self.throttler.id() => self.throttler.update(),
            id if id == self.throttler.fid() => self.flush_blocked_peers(),
            _ => self.handle_peer_ev(not),
        }
    }

    fn handle_trk_ev(&mut self) {
        loop {
            match self.trk_rx.try_recv() {
                Ok(resp) => {
                    for ip in resp.peers.iter() {
                        if let Ok(peer) = Peer::new_outgoing(ip) {
                            self.add_peer(resp.id, peer);
                        }
                    }
                }
                Err(_) => { break; }
            }
        }
    }

    fn handle_disk_ev(&mut self) {
        loop {
            match self.disk_rx.try_recv() {
                Ok(resp) => {
                    let pid = resp.context.id;
                    let tid = self.peers[&pid];
                    let ref mut torrent = self.torrents.get_mut(&tid).unwrap();
                    torrent.block_available(pid, resp).unwrap();
                }
                Err(_) => { break; }
            }
        }
    }

    fn handle_ctrl_ev(&mut self) {
        loop {
            match self.ctrl_rx.try_recv() {
                Ok(Request::AddTorrent(b)) => {
                    if let Ok(i) = torrent::Info::from_bencode(b) {
                        let r = self.reg.clone();
                        let t = self.throttler.get_throttle();
                        self.add_torrent(Torrent::new(i, t, r));
                    }
                }
                Ok(Request::AddPeer(p, hash)) => {
                    let tid = *self.hash_idx.get(&hash).unwrap();
                    self.add_peer(tid, p);
                }
                Ok(Request::RPC(r)) => {
                    self.handle_rpc(r);
                }
                Err(_) => { break; }
            }
        }
    }

    fn handle_peer_ev(&mut self, not: amy::Notification) {
        let pid = not.id;
        if not.event.readable() {
            let res = {
                let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
                torrent.peer_readable(pid)
            };
            if res.is_err() {
                self.remove_peer(pid);
                return;
            }
        }
        if not.event.writable() {
            let res = {
                let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
                torrent.peer_writable(pid)
            };
            if res.is_err() {
                self.remove_peer(pid);
            }
        }
    }

    fn flush_blocked_peers(&mut self) {
        let pids = self.throttler.flush();
        for pid in  pids {
            let res = {
                let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
                torrent.peer_readable(pid)
            };
            if res.is_err() {
                self.remove_peer(pid);
            }
        }
    }

    fn add_torrent(&mut self, mut t: Torrent) {
        let tid = self.tid_cnt;
        t.id = tid;
        TRACKER.tx.send(tracker::Request::started(&t)).unwrap();
        self.hash_idx.insert(t.info.hash, tid);
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
                    RPC.tx.send(rpc::Response::Err("Torrent ID not found!")).unwrap();
                }
            }
            rpc::Request::AddTorrent(data) => {
                match torrent::Info::from_bencode(data) {
                    Ok(i) => {
                        let r = self.reg.clone();
                        let t = self.throttler.get_throttle();
                        self.add_torrent(Torrent::new(i, t, r));
                        RPC.tx.send(rpc::Response::Ack).unwrap();
                    }
                    Err(e) => {
                        RPC.tx.send(rpc::Response::Err(e)).unwrap();
                    }
                }
            }
            rpc::Request::StopTorrent(id) => {

            }
            rpc::Request::StartTorrent(id) => {

            }
            rpc::Request::RemoveTorrent(id) => {

            }
            rpc::Request::ThrottleUpload(amnt) => {

            }
            rpc::Request::ThrottleDownload(amnt) => {

            }
        }
    }

    fn add_peer(&mut self, id: usize, peer: Peer) {
        let torrent = self.torrents.get_mut(&id).unwrap();
        if let Some(pid) = torrent.add_peer(peer) {
            self.peers.insert(pid, id);
        }
    }

    fn remove_peer(&mut self, id: usize) {
        println!("Removing peer {:?}", id);
        let tid = self.peers.remove(&id).expect("Removed pid should be in peer map!");
        let torrent = self.torrents.get_mut(&tid).expect("Torrent should be present in map");
        torrent.remove_peer(id);
    }
}

pub fn start() -> Handle {
    let poll = amy::Poller::new().unwrap();
    let mut reg = poll.get_registrar().unwrap();
    let (trk_tx, trk_rx) = reg.channel().unwrap();
    let (disk_tx, disk_rx) = reg.channel().unwrap();
    let (ctrl_tx, ctrl_rx) = reg.channel().unwrap();
    thread::spawn(move || {
        Control::new(poll, trk_rx, disk_rx, ctrl_rx).run();
    });
    Handle { trk_tx: Mutex::new(trk_tx), disk_tx: Mutex::new(disk_tx), ctrl_tx: Mutex::new(ctrl_tx) }
}
