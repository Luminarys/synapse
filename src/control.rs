use std::thread;
use {tracker, disk, TRACKER};
use mio::{channel, Event, Events, Poll, PollOpt, Ready, Token};
use torrent::{Torrent, Peer};
use slab::Slab;
use std::sync::mpsc::TryRecvError;
use util::{tok_enc, tok_dec};

pub struct Control {
    trk_rx: channel::Receiver<tracker::Response>,
    disk_rx: channel::Receiver<disk::Response>,
    ctrl_rx: channel::Receiver<Request>,
    poll: Poll,
    torrents: Slab<Torrent, usize>,
}

pub struct Handle {
    pub trk_tx: channel::Sender<tracker::Response>,
    pub disk_tx: channel::Sender<disk::Response>,
    pub ctrl_tx: channel::Sender<Request>,
}

impl Handle {
    pub fn trk_tx(&self) -> channel::Sender<tracker::Response> {
        self.trk_tx.clone()
    }

    pub fn disk_tx(&self) -> channel::Sender<disk::Response> {
        self.disk_tx.clone()
    }

    pub fn ctrl_tx(&self) -> channel::Sender<Request> {
        self.ctrl_tx.clone()
    }
}

unsafe impl Sync for Handle {}

pub enum Request {
    AddTorrent(Torrent),
}

const TRK_RX: Token = Token(1 << 63);
const DISK_RX: Token = Token(1 << 63 | 1);
const CTRL_RX: Token = Token(1 << 63 | 2);

impl Control {
    pub fn new(trk_rx: channel::Receiver<tracker::Response>,
               disk_rx: channel::Receiver<disk::Response>,
               ctrl_rx: channel::Receiver<Request>) -> Control {
        let poll = Poll::new().unwrap();
        let torrents = Slab::with_capacity(128);
        poll.register(&trk_rx, TRK_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        poll.register(&disk_rx, DISK_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        poll.register(&ctrl_rx, CTRL_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        Control { trk_rx, disk_rx, ctrl_rx, poll, torrents }
    }

    pub fn run(&mut self) {
        let mut events = Events::with_capacity(256);
        loop {
            self.poll.poll(&mut events, None).unwrap();
            for event in events.iter() {
                self.handle_event(event);
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event.token() {
            TRK_RX => self.handle_trk_ev(),
            DISK_RX => self.handle_disk_ev(event),
            CTRL_RX => self.handle_ctrl_ev(),
            _ => self.handle_peer_ev(event),
        }
    }

    fn handle_trk_ev(&mut self) {
        loop {
            match self.trk_rx.try_recv() {
                Ok(mut resp) => {
                    let ref mut torrent = self.torrents.get_mut(resp.id).unwrap();
                    resp.peers.push("127.0.0.1:8999".parse().unwrap());
                    for ip in resp.peers.iter() {
                        let peer = Peer::new_outgoing(ip, &torrent.info).unwrap();
                        let pid = torrent.insert_peer(peer).unwrap();
                        let tok = Token(tok_enc(resp.id, pid));
                        self.poll.register(&torrent.get_peer_mut(pid).unwrap().conn, tok, Ready::all(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
                    }
                }
                Err(TryRecvError::Empty) => { break; }
                _ => { unreachable!(); }
            }
        }
        self.poll.reregister(&self.trk_rx, TRK_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
    }

    fn handle_disk_ev(&mut self, event: Event) {
        loop {
            match self.disk_rx.try_recv() {
                Ok(resp) => {
                    let (tid, pid) = tok_dec(event.token().0);
                    let ref mut torrent = self.torrents.get_mut(tid).unwrap();
                    torrent.block_available(pid, resp).unwrap();
                }
                Err(TryRecvError::Empty) => { break; }
                _ => { unreachable!(); }
            }
        }
        self.poll.reregister(&self.disk_rx, DISK_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
    }

    fn handle_ctrl_ev(&mut self) {
        loop {
            match self.ctrl_rx.try_recv() {
                Ok(Request::AddTorrent(t)) => {
                    let tid = self.torrents.insert(t).unwrap();
                    let ref mut torrent = self.torrents.get_mut(tid).unwrap();
                    torrent.id = tid;
                    TRACKER.tx.send(tracker::Request::new(tid, 5678, torrent, tracker::Event::Started)).unwrap();
                }
                Err(TryRecvError::Empty) => { break; }
                _ => { unreachable!(); }
            }
        }
        self.poll.reregister(&self.ctrl_rx, CTRL_RX, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
    }

    fn handle_peer_ev(&mut self, event: Event) {
        let (tid, pid) = tok_dec(event.token().0);
        let mut torrent = self.torrents.get_mut(tid).unwrap();
        let mut ready = Ready::readable() | Ready::writable();
        if event.readiness().is_readable() {
            if let Err(e) = torrent.peer_readable(pid) {
                println!("Peer {:?} error'd with {:?}, removing", pid, e);
                torrent.remove_peer(pid);
                return;
            }
        }
        if event.readiness().is_writable() {
            match torrent.peer_writable(pid) {
                Ok(false) => ready.remove(Ready::writable()),
                Ok(true) => { }
                Err(e) => {
                    println!("Peer {:?} error'd with {:?}, removing", pid, e);
                    torrent.remove_peer(pid);
                    return;
                }
            }
        }
        self.poll.reregister(&torrent.get_peer_mut(pid).unwrap().conn, event.token(), ready, PollOpt::edge() | PollOpt::oneshot()).unwrap();
    }
}

pub fn start() -> Handle {
    let (trk_tx, trk_rx) = channel::channel();
    let (disk_tx, disk_rx) = channel::channel();
    let (ctrl_tx, ctrl_rx) = channel::channel();
    thread::spawn(move || {
        Control::new(trk_rx, disk_rx, ctrl_rx).run();
    });
    Handle { trk_tx, disk_tx, ctrl_tx }
}
