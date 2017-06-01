use std::thread;
use {rpc, tracker, disk, TRACKER, RPC};
use amy::{self, Poller, Registrar};
use torrent::{Torrent, Peer};
use torrent::peer::Message;
use std::collections::HashMap;
use std::fmt;

pub struct Control {
    trk_rx: amy::Receiver<tracker::Response>,
    disk_rx: amy::Receiver<disk::Response>,
    ctrl_rx: amy::Receiver<Request>,
    reg: Registrar,
    poll: Poller,
    tid_cnt: usize,
    torrents: HashMap<usize, Torrent>,
    peers: HashMap<usize, usize>,
    hash_idx: HashMap<[u8; 20], usize>,
}

pub struct Handle {
    trk_tx: amy::Sender<tracker::Response>,
    disk_tx: amy::Sender<disk::Response>,
    ctrl_tx: amy::Sender<Request>,
}

impl Handle {
    pub fn trk_tx(&self) -> amy::Sender<tracker::Response> {
        self.trk_tx.try_clone().unwrap()
    }

    pub fn disk_tx(&self) -> amy::Sender<disk::Response> {
        self.disk_tx.try_clone().unwrap()
    }

    pub fn ctrl_tx(&self) -> amy::Sender<Request> {
        self.ctrl_tx.try_clone().unwrap()
    }
}

unsafe impl Sync for Handle {}

pub enum Request {
    AddTorrent(Torrent),
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
        let reg = poll.get_registrar().unwrap();
        Control { trk_rx, disk_rx, ctrl_rx, poll, torrents, peers, hash_idx, reg, tid_cnt: 0 }
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
                Ok(Request::AddTorrent(t)) => {
                    self.add_torrent(t);
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
            match res {
                Ok(_) => { }
                Err(_) => {
                    println!("Peer {:?} error, removing", pid);
                    self.remove_peer(pid);
                    return;
                }
            }
        }
        if not.event.writable() {
            if let Err(_) = {
                let torrent = self.torrents.get_mut(&self.peers[&pid]).unwrap();
                torrent.peer_writable(pid)
            } {
                println!("Peer {:?} error, removing", pid);
                self.remove_peer(pid);
                return;
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
                match Torrent::from_bencode(data) {
                    Ok(t) => {
                        self.add_torrent(t);
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

    fn add_peer(&mut self, id: usize, mut peer: Peer) {
        let torrent = self.torrents.get_mut(&id).unwrap();
        let pid = self.reg.register(&peer.conn, amy::Event::Both).unwrap();
        peer.id = pid;
        match torrent.add_peer(peer) {
            Err(e) => {
                println!("Error {:?}", e);
            }
            _ => {
                self.peers.insert(pid, id);
            }
        };
    }

    fn remove_peer(&mut self, id: usize) {
        let tid = self.peers.remove(&id).unwrap();
        let torrent = self.torrents.get_mut(&tid).unwrap();
        let peer = torrent.remove_peer(id);
        self.reg.deregister(&peer.conn).unwrap();
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
    Handle { trk_tx, disk_tx, ctrl_tx }
}
