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
    peers: Slab<Peer>,
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
        let poll = Poll::new().unwrap();
        poll.register(&self.manager_rx, MANAGER_TOK, Ready::readable(), PollOpt::level());

        let mut events = Events::with_capacity(1024);
        loop {
            poll.poll(&mut events, None).unwrap();
            for event in events.iter() {
                match event.token() {
                    MANAGER_TOK => {
                        if self.handle_manager_ev() {
                            self.cleanup();
                            break;
                        }
                    }
                    _ => {
                    }
                }
            }
        }
    }

    fn handle_manager_ev(&mut self) -> bool {
        match self.manager_rx.try_recv() {
            Ok(WorkerReq::Shutdown) => {
                return true;
            }
            Ok(WorkerReq::NewConn { id, hash, peer }) => {
                if !self.peers.has_available() {
                    self.kill_peer_conn();
                }
                let entry = self.peers.vacant_entry().unwrap();
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
