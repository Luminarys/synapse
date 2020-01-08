use std::time::{Duration, Instant};

use control::cio;
use torrent::Peer;
use util::{random_sample, FHashSet, UHashMap};

pub struct Choker {
    unchoked: Vec<usize>,
    interested: FHashSet<usize>,
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
            interested: FHashSet::default(),
            last_updated: Instant::now(),
        }
    }

    pub fn add_peer<T: cio::CIO>(&mut self, peer: &mut Peer<T>) {
        if self.unchoked.len() < 5 {
            self.unchoked.push(peer.id());
            peer.flush();
            peer.unchoke();
        } else {
            self.interested.insert(peer.id());
        }
    }

    fn unchoke_random<T: cio::CIO>(&mut self, peers: &mut UHashMap<Peer<T>>) -> Option<usize> {
        if let Some(random_id) = random_sample(self.interested.iter()).cloned() {
            peers.get_mut(&random_id).map(|mut peer| {
                self.interested.remove(&random_id);
                self.add_peer(&mut peer);
                random_id
            })
        } else {
            None
        }
    }

    pub fn remove_peer<T: cio::CIO>(
        &mut self,
        peer: &mut Peer<T>,
        peers: &mut UHashMap<Peer<T>>,
    ) -> Option<SwapRes> {
        if let Some(idx) = self.unchoked.iter().position(|&id| id == peer.id()) {
            self.unchoked.remove(idx);
            peer.choke();
            self.unchoke_random(peers).map(|unchoked| SwapRes {
                choked: peer.id(),
                unchoked,
            })
        } else {
            self.interested.remove(&peer.id());
            None
        }
    }

    fn update_timer(&mut self) -> Result<(), ()> {
        if self.last_updated.elapsed() < Duration::from_secs(10)
            || self.unchoked.len() < 5
            || self.interested.is_empty()
        {
            Err(())
        } else {
            self.last_updated = Instant::now();
            Ok(())
        }
    }

    pub fn update_upload<T: cio::CIO>(&mut self, peers: &mut UHashMap<Peer<T>>) -> Option<SwapRes> {
        if self.update_timer().is_err() {
            return None;
        }
        if self.interested.is_empty() {
            return None;
        }
        let (slowest, _) = self.unchoked.iter().enumerate().fold(
            (0, ::std::u32::MAX),
            |(slowest, min), (idx, id)| {
                match peers.get_mut(id).map(Peer::flush) {
                    Some((ul, _)) if ul < min => (idx, ul),
                    _ => (slowest, min),
                }
            }
        );
        Some(self.swap_peer(slowest, peers))
    }

    pub fn update_download<T: cio::CIO>(
        &mut self,
        peers: &mut UHashMap<Peer<T>>,
    ) -> Option<SwapRes> {
        if self.update_timer().is_err() {
            return None;
        }

        let (slowest, _) = self.unchoked.iter().enumerate().fold(
            (0, ::std::u32::MAX),
            |(slowest, min), (idx, id)| {
                match peers.get_mut(id).map(Peer::flush) {
                    Some((_, dl)) if dl < min =>
                        (idx, dl),
                    _ =>
                        (slowest, min),
                }
            },
        );
        Some(self.swap_peer(slowest, peers))
    }

    fn swap_peer<T: cio::CIO>(&mut self, idx: usize, peers: &mut UHashMap<Peer<T>>) -> SwapRes {
        let id = self.unchoked.remove(idx);
        {
            peers.get_mut(&id).map(Peer::choke);
        }

        // Unchoke one random interested peer
        let r = SwapRes {
            choked: id,
            unchoked: self.unchoke_random(peers).unwrap(),
        };
        self.interested.insert(id);
        r
    }
}

#[cfg(test)]
mod tests {
    use super::{Choker, SwapRes};
    use std::time::{Duration, Instant};
    use torrent::{Bitfield, Peer};
    use util::UHashMap;

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
        let mut h = UHashMap::default();
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, 0, 0);
            c.add_peer(&mut p);
            v.push(p);
            // Semi copy
            let pc = Peer::test_from_stats(i, 0, 0);
            h.insert(i, pc);
        }
        assert_eq!(c.unchoked.contains(&v[0].id()), true);
        assert_eq!(
            c.remove_peer(&mut v[0], &mut h),
            Some(SwapRes {
                choked: v[0].id(),
                unchoked: 5,
            })
        );
        assert_eq!(c.unchoked.contains(&v[0].id()), false);
    }

    #[test]
    fn test_update_upload() {
        let mut c = Choker::new();
        let mut h = UHashMap::default();
        assert_eq!(c.update_upload(&mut h).is_none(), true);
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, i as u32, 6 - i as u32);
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
        let mut h = UHashMap::default();
        assert_eq!(c.update_download(&mut h).is_none(), true);
        for i in 0..6 {
            let mut p = Peer::test_from_stats(i, 6 - i as u32, i as u32);
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
