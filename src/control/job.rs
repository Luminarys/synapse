use std::collections::HashSet;
use std::net::SocketAddr;
use std::time;

use crate::control::cio;
use crate::torrent::Torrent;
use crate::util::UHashMap;

pub trait Job<T: cio::CIO> {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>);
}

pub struct TrackerUpdate;

impl<T: cio::CIO> Job<T> for TrackerUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.try_update_tracker();
        }
    }
}

pub struct UnchokeUpdate;

impl<T: cio::CIO> Job<T> for UnchokeUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_unchoked();
        }
    }
}

pub struct SessionUpdate;

impl<T: cio::CIO> Job<T> for SessionUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            if torrent.dirty() {
                torrent.serialize();
            }
        }
    }
}

pub struct TorrentTxUpdate {
    piece_update: time::Instant,
    active: UHashMap<bool>,
}

impl TorrentTxUpdate {
    pub fn new() -> TorrentTxUpdate {
        TorrentTxUpdate {
            piece_update: time::Instant::now(),
            active: UHashMap::default(),
        }
    }
}

impl<T: cio::CIO> Job<T> for TorrentTxUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (id, torrent) in torrents.iter_mut() {
            let active = torrent.tick();
            if active {
                torrent.update_rpc_transfer();
                torrent.update_rpc_peers();
                // TODO: consider making tick triggered by on the fly validation
                if self.piece_update.elapsed() > time::Duration::from_secs(30) {
                    torrent.rpc_update_pieces();
                    self.piece_update = time::Instant::now();
                }
            }
            if !torrent.complete() {
                torrent.rank_peers();
            }

            if !self.active.contains_key(id) {
                self.active.insert(*id, active);
            }
            let prev = self.active.get_mut(id).unwrap();
            if *prev != active {
                *prev = active;
                torrent.announce_status();
            }
        }
        self.active.retain(|id, _| torrents.contains_key(id));
    }
}

pub struct PEXUpdate {
    peers: UHashMap<HashSet<SocketAddr>>,
}

impl PEXUpdate {
    pub fn new() -> PEXUpdate {
        PEXUpdate {
            peers: UHashMap::default(),
        }
    }
}

impl<T: cio::CIO> Job<T> for PEXUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (id, torrent) in torrents.iter_mut().filter(|&(_, ref t)| !t.info().private) {
            if !self.peers.contains_key(id) {
                self.peers.insert(*id, HashSet::new());
            }

            let (added, removed) = {
                let peers: HashSet<_> = torrent.peers().values().map(|p| p.addr()).collect();
                let prev = self.peers.get_mut(id).unwrap();
                let mut add: Vec<_> = peers.difference(prev).cloned().collect();
                let mut rem: Vec<_> = prev.difference(&peers).cloned().collect();
                add.truncate(50);
                rem.truncate(50 - add.len());
                (add, rem)
            };
            torrent.update_pex(&added, &removed);
        }
        self.peers.retain(|id, _| torrents.contains_key(id));
    }
}
