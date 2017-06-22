use std::collections::{HashSet, HashMap};
use std::time::{Instant, Duration};
use std::{self, io};
use torrent::Peer;
use util::random_sample;

pub struct Choker {
    unchoked: Vec<usize>,
    interested: HashSet<usize>,
    last_updated: Instant,
}

pub struct SwapRes {
    pub choked: (usize, io::Result<()>),
    pub unchoked: (usize, io::Result<()>),
}

impl Choker {
    pub fn new() -> Choker {
        Choker {
            unchoked: Vec::with_capacity(5),
            interested: HashSet::new(),
            last_updated: Instant::now()
        }
    }

    pub fn add_peer(&mut self, peer: &mut Peer) -> io::Result<()> {
        if self.unchoked.len() < 5 {
            self.unchoked.push(peer.id);
            peer.downloaded = 0;
            peer.uploaded = 0;
            peer.unchoke()
        } else {
            self.interested.insert(peer.id);
            Ok(())
        }
    }

    pub fn remove_peer(&mut self, peer: &mut Peer) {
        if self.unchoked.contains(&peer.id) {
        } else {
            self.interested.remove(&peer.id);
        }
    }

    fn update_timer(&mut self) -> Result<(), ()> {
        if self.last_updated.elapsed() < Duration::from_secs(10) || self.unchoked.len() < 5 {
            Err(())
        } else {
            self.last_updated = Instant::now();
            Ok(())
        }
    }

    pub fn update_upload(&mut self, peers: &mut HashMap<usize, Peer>) -> Option<SwapRes> {
        if self.update_timer().is_err() {
            return None;
        }
        let (slowest, _) = self.unchoked.iter().enumerate().fold((0, std::usize::MAX), |(slowest, min), (idx, id)| {
            let ul = peers[id].uploaded;
            peers.get_mut(id).unwrap().uploaded = 0;
            if ul < min {
                (idx, ul)
            } else {
                (slowest, min)
            }
        });
        Some(self.swap_peer(slowest, peers))
    }

    pub fn update_download(&mut self, peers: &mut HashMap<usize, Peer>) -> Option<SwapRes> {
        if self.update_timer().is_err() {
            return None;
        }

        let (slowest, _) = self.unchoked.iter().enumerate().fold((0, std::usize::MAX), |(slowest, min), (idx, id)| {
            let ul = peers[id].downloaded;
            peers.get_mut(id).unwrap().downloaded = 0;
            if ul < min {
                (idx, ul)
            } else {
                (slowest, min)
            }
        });
        Some(self.swap_peer(slowest, peers))
    }

    fn swap_peer(&mut self, idx: usize, peers: &mut HashMap<usize, Peer>) -> SwapRes {
        let id = self.unchoked.remove(idx);
        let cres = {
            let peer = peers.get_mut(&id).unwrap();
            self.interested.insert(id);
            peer.choke()
        };

        // Unchoke one random interested peer
        let random_id = *random_sample(self.interested.iter()).unwrap();
        let peer = peers.get_mut(&random_id).unwrap();
        self.interested.remove(&random_id);
        self.unchoked.push(random_id);
        let ures = peer.unchoke();
        return SwapRes { choked: (id, cres), unchoked: (random_id, ures) };
    }
}

mod tests {
    use super::Choker;
    use torrent::Peer;
    use socket::Socket;

    #[test]
    fn test_add_peers() {
        let mut c = Choker::new();
        for i in 0..6 {
            let mut p = Peer::new(Socket::empty());
            p.id = i;
            c.add_peer(&mut p).is_err();
        }
        assert_eq!(c.unchoked.len(), 5);
        assert_eq!(c.interested.len(), 1);
    }
}
