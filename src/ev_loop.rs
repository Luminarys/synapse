use std::io;
use mio::{channel, Events, Poll, PollOpt, Ready, Token};
use slab::Slab;
use torrent::Torrent;

enum Handle {
    Peer(usize),
    Tracker(usize),
    Incoming(usize),
    Listener,
}

pub struct EvLoop {
    poll: Poll,
    handles: Slab<Handle, Token>,
    torrents: Slab<Torrent, usize>,
    incoming: Slab<(), Token>,
}

impl EvLoop {
    pub fn new() -> Result<EvLoop, io::Error> {
        unimplemented!();
    }

    pub fn run(&mut self) -> Result<(), io::Error> {
        Ok(())
    }

    pub fn add_torrent(&mut self, torrent: Torrent) {
    
    }
}
