use std::sync::mpsc;
use std::collections::HashMap;
use mio::{channel, Events, Poll, PollOpt, Ready, Token};
use mio::tcp::TcpStream;
use piece_field::PieceField;
use peer::{IncomingPeer, Peer};
use torrent::{Torrent, TorrentInfo};
use slab::Slab;
use manager;

pub enum WorkerReq {
    NewConn{ id: [u8; 20], hash: [u8; 20], peer: IncomingPeer },
    Shutdown,
    NewTorrent(TorrentInfo)
}

pub enum WorkerResp {
}

pub struct Worker {
    manager_tx: channel::Sender<WorkerResp>,
    manager_rx: channel::Receiver<WorkerReq>,
    peers: Slab<Peer, Token>,
    torrents: HashMap<[u8; 20], Torrent>,
}

impl Worker {
    pub fn new(mtx: channel::Sender<WorkerResp>) -> (Worker, channel::Sender<WorkerReq>) {
        let (tx, rx) = channel::channel();
        (Worker {
            manager_tx: mtx,
            manager_rx: rx,
            peers: Slab::with_capacity(10_000),
            torrents: HashMap::new(),
        }, tx)
    }

    pub fn run(&mut self) {
        const MANAGER_TOK: Token = Token(1_000_000);
        let mut poll = Poll::new().unwrap();
        poll.register(&self.manager_rx, MANAGER_TOK, Ready::readable(), PollOpt::level());

        let mut events = Events::with_capacity(1024);
        loop {
            poll.poll(&mut events, None).unwrap();
            for event in events.iter() {
                match event.token() {
                    MANAGER_TOK => {
                        if self.handle_manager_ev(&mut poll) {
                            self.cleanup();
                            break;
                        }
                    }
                    tok => {
                        if event.kind().is_hup() || event.kind().is_error() {
                            self.peers.remove(tok);
                        } else {
                            let peer = self.peers.get_mut(tok).unwrap();
                            let torrent = self.torrents.get_mut(&peer.data.torrent).unwrap();
                            if event.kind().is_readable() {
                                peer.readable(torrent);
                            }
                            if event.kind().is_writable() {
                                peer.writable(torrent);
                            }
                            poll.reregister(peer.socket(), tok, Ready::all(), PollOpt::edge()).unwrap();
                        }
                    }
                }
            }
        }
    }

    fn handle_manager_ev(&mut self, poll: &mut Poll) -> bool {
        match self.manager_rx.try_recv() {
            Ok(WorkerReq::Shutdown) => {
                return true;
            }
            Ok(WorkerReq::NewConn { id, hash, mut peer }) => {
                if !self.peers.has_available() {
                    self.kill_peer_conn();
                }
                let entry = self.peers.vacant_entry().unwrap();
                let tok = entry.index();
                // We unwrap the entry in torrents because we know that if
                // the manager thread let this msg go through it must be for
                // a valid torrent.
                let peer = Peer::new_client(peer, id, self.torrents.get(&hash).unwrap());
                poll.register(peer.socket(), tok, Ready::all(), PollOpt::edge()).unwrap();
                entry.insert(peer);
            }
            _ => { }
        };
        false
    }

    /// Kills a peer conn, prioritizing idle connections
    fn kill_peer_conn(&mut self) {
    }

    fn cleanup(&mut self) {
    
    }
}
