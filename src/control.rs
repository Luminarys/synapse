use std::thread;
use {tracker, disk, TRACKER};
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
    peers: HashMap<usize, Peer>,
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
    AddPeer(Peer, [u8; 20], Vec<Message>),
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
                Ok(mut resp) => {
                    let ref mut torrent = self.torrents.get_mut(&resp.id).unwrap();
                    // resp.peers.push("127.0.0.1:8999".parse().unwrap());
                    for ip in resp.peers.iter() {
                        if let Ok(mut peer) = Peer::new_outgoing(ip, &torrent) {
                            let pid = self.reg.register(&peer.conn, amy::Event::Both).unwrap();
                            peer.id = pid;
                            self.peers.insert(pid, peer);
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
                    let ref mut peer = self.peers.get_mut(&pid).unwrap();
                    let ref mut torrent = self.torrents.get_mut(&peer.tid).unwrap();
                    torrent.block_available(peer, resp).unwrap();
                }
                Err(_) => { break; }
            }
        }
    }

    fn handle_ctrl_ev(&mut self) {
        loop {
            match self.ctrl_rx.try_recv() {
                Ok(Request::AddTorrent(mut t)) => {
                    let tid = self.tid_cnt;
                    t.id = tid;
                    TRACKER.tx.send(tracker::Request::new(tid, 5678, &t, tracker::Event::Started)).unwrap();
                    self.hash_idx.insert(t.info.hash, tid);

                    self.tid_cnt += 1;
                    self.torrents.insert(tid, t);
                }
                Ok(Request::AddPeer(mut p, hash, msgs)) => {
                    let tid = self.hash_idx.get(&hash).unwrap();
                    let ref mut torrent = self.torrents.get_mut(tid).unwrap();
                    p.set_torrent(torrent).unwrap();
                    let pid = self.reg.register(&p.conn, amy::Event::Both).unwrap();
                    p.id = pid;
                    let mut err = false;
                    for msg in msgs {
                        if let Err(_) = torrent.handle_msg(msg, &mut p) {
                            err = true;
                            break;
                        }
                    }
                    if !err {
                        self.peers.insert(pid, p);
                    }
                }
                Err(_) => { break; }
            }
        }
    }

    fn handle_peer_ev(&mut self, not: amy::Notification) {
        let pid = not.id;
        if not.event.readable() {
            if let Err(e) = {
                let peer = self.peers.get_mut(&pid).unwrap();
                let torrent = self.torrents.get_mut(&peer.tid).unwrap();
                torrent.peer_readable(peer)
            } {
                println!("Peer {:?} error'd with {:?}, removing", pid, e);
                self.peers.remove(&pid);
                return;
            }
        }
        if not.event.writable() {
            if let Err(e) = {
                let peer = self.peers.get_mut(&pid).unwrap();
                let torrent = self.torrents.get_mut(&peer.tid).unwrap();
                torrent.peer_writable(peer)
            } {
                println!("Peer {:?} error'd with {:?}, removing", pid, e);
                self.peers.remove(&pid);
                return;
            }
        }
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
