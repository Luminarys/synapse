use std::collections::{HashSet, HashMap};
use std::time::{Instant, Duration};
use std;
use torrent::Peer;
use util::random_sample;

pub struct Choker {
    unchoked: Vec<usize>,
    interested: HashSet<usize>,
    last_updated: Instant,
}

#[derive(Debug, PartialEq)]
pub struct SwapRes {
    pub choked: usize,
    pub unchoked: usize,
}


impl Choker {
    pub fn new() -> Choker {
        Choker {
            unchoked: Vec::with_capacity(5),
            interested: HashSet::new(),
            last_updated: Instant::now()
        }
    }

    pub fn add_peer(&mut self, peer: &mut Peer) {
        if self.unchoked.len() < 5 {
            self.unchoked.push(peer.id());
            peer.flush_dl();
            peer.flush_ul();
            peer.unchoke();
        } else {
            self.interested.insert(peer.id());
        }
    }

    fn unchoke_random(&mut self, peers: &mut HashMap<usize, Peer>) -> Option<usize> {
        if let Some(random_id) = random_sample(self.interested.iter()).cloned() {
            let mut peer = peers.get_mut(&random_id).unwrap();
            self.interested.remove(&random_id);
            self.add_peer(&mut peer);
            Some(random_id)
        } else {
            None
        }
    }

    pub fn remove_peer(&mut self, peer: &mut Peer, peers: &mut HashMap<usize, Peer>) -> Option<SwapRes> {
        if let Some(idx) = self.unchoked.iter().position(|&id| id == peer.id()) {
            self.unchoked.remove(idx);
            peer.choke();
            self.unchoke_random(peers).map(|unchoked| SwapRes { choked: peer.id(), unchoked })
        } else {
            self.interested.remove(&peer.id());
            None
        }
    }

    fn update_timer(&mut self) -> Result<(), ()> {
        if self.last_updated.elapsed() < Duration::from_secs(10) || self.unchoked.len() < 5 || self.interested.is_empty() {
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
        if self.interested.is_empty() {
            return None;
        }
        let (slowest, _) = self.unchoked.iter().enumerate().fold((0, std::usize::MAX), |(slowest, min), (idx, id)| {
            let ul = peers.get_mut(id).unwrap().flush_ul();
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
            let dl = peers.get_mut(id).unwrap().flush_dl();
            if dl < min {
                (idx, dl)
            } else {
                (slowest, min)
            }
        });
        Some(self.swap_peer(slowest, peers))
    }

    fn swap_peer(&mut self, idx: usize, peers: &mut HashMap<usize, Peer>) -> SwapRes {
        let id = self.unchoked.remove(idx);
        {
            let peer = peers.get_mut(&id).unwrap();
            peer.choke();
        }

        // Unchoke one random interested peer
        let r = SwapRes { choked: id, unchoked: self.unchoke_random(peers).unwrap() };
        self.interested.insert(id);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::{Choker, SwapRes};
    use torrent::{Peer, Bitfield};
    use time::{Instant, Duration};
    use std::collections::HashMap;

    #[test]
    fn test_add_peers() {
        let mut c = Choker::new();
        for i in 0..6 {
            let mut p = Peer::test(i, 0, 0, 0, Bitfield::new(1));
            // Since the socket is a dummy
            c.add_peer(&mut p);
        }
        assert_eq!(c.unchoked.len(), 5);
        assert_eq!(c.interested.len(), 1);
    }

    #[test]
    fn test_remove_peers() {
        let mut c = Choker::new();
        let mut v = Vec::new();
        let mut h = HashMap::new();
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, 0, 0);
            c.add_peer(&mut p);
            v.push(p);
            // Semi copy
            let pc = Peer::test_from_stats(i, 0, 0);
            h.insert(i, pc);
        }
        assert_eq!(c.unchoked.contains(&v[0].id()), true);
        assert_eq!(c.remove_peer(&mut v[0], &mut h), Some(SwapRes { choked: v[0].id(), unchoked: 5}));
        assert_eq!(c.unchoked.contains(&v[0].id()), false);
    }

    #[test]
    fn test_update_upload() {
        let mut c = Choker::new();
        let mut h = HashMap::new();
        assert_eq!(c.update_upload(&mut h).is_none(), true);
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, i, 6 - i);
            c.add_peer(&mut p);
            h.insert(i, p);
        }
        assert_eq!(c.update_upload(&mut h).is_none(), true);
        c.last_updated = Instant::now() - Duration::from_secs(11);
        let res = c.update_upload(&mut h).unwrap();
        assert_eq!(res.choked, 0);
        assert_eq!(res.unchoked, 5);
    }

    #[test]
    fn test_update_download() {
        let mut c = Choker::new();
        let mut h = HashMap::new();
        assert_eq!(c.update_download(&mut h).is_none(), true);
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, 6 - i, i);
            c.add_peer(&mut p);
            h.insert(i, p);
        }
        assert_eq!(c.update_download(&mut h).is_none(), true);
        c.last_updated = Instant::now() - Duration::from_secs(11);
        let res = c.update_download(&mut h).unwrap();
        assert_eq!(res.choked, 0);
        assert_eq!(res.unchoked, 5);
    }
}
