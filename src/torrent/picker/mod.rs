use std::collections::HashSet;
use torrent::{Info, Peer};

mod rarest;
mod sequential;

pub enum Picker {
    Rarest(rarest::Picker),
    Sequential(sequential::Picker),
}

impl Picker {
    pub fn new_rarest(info: &Info) -> Picker {
        let picker = rarest::Picker::new(info);
        Picker::Rarest(picker)
    }

    pub fn new_sequential(info: &Info) -> Picker {
        let picker = sequential::Picker::new(info);
        Picker::Sequential(picker)
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        match *self {
            Picker::Sequential(ref mut p) => p.pick(peer),
            Picker::Rarest(ref mut p) => p.pick(peer),
        }
    }

    /// Returns whether or not the whole piece is complete.
    pub fn completed(&mut self, idx: u32, offset: u32) -> (bool, HashSet<usize>) {
        match *self {
            Picker::Sequential(ref mut p) => p.completed(idx, offset),
            Picker::Rarest(ref mut p) => p.completed(idx, offset),
        }
    }
}
