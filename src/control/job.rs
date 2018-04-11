use std::time;

use torrent::Torrent;
use control::cio;
use util::UHashMap;

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
