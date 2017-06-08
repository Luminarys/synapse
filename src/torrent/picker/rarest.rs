use std::collections::{HashSet, HashMap};
use torrent::{Bitfield, Info, Peer};

pub struct Picker {

}

impl Picker {
    pub fn new(info: &Info) -> Picker {
        Picker { }
    }

    pub fn pick(&mut self, peer: &Peer) -> Option<(u32, u32)> {
        unimplemented!();
    }

    pub fn completed(&mut self, mut idx: u32, mut offset: u32) -> (bool, HashSet<usize>) {
        unimplemented!();
    }
}
