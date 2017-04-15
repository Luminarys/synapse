use std::io;
use mio::{channel, Event, Events, Poll, PollOpt, Ready, Token};
use mio::tcp::{TcpListener, TcpStream};
use slab::Slab;
use torrent::Torrent;

enum Handle {
    Peer { torrent: usize, pid: [u8; 20] },
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
    pub fn new() -> io::Result<EvLoop> {
        let poll = Poll::new()?;
        let handles = Slab::with_capacity(128);
        let torrents = Slab::with_capacity(128);
        let incoming = Slab::with_capacity(128);

        Ok(EvLoop {
            poll: poll,
            handles: handles,
            torrents: torrents,
            incoming: incoming,
        })
    }

    pub fn run(&mut self) -> io::Result<()> {
        let mut events = Events::with_capacity(256);
        loop {
            self.poll.poll(&mut events, None)?;
            for event in events.iter() {
                match self.handle_event(event) {
                    Err(e) => {
                    
                    }
                    _ => (),
                }
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        let handle = self.handles.get_mut(event.token()).unwrap();
        match *handle {
            Handle::Peer { torrent, pid } => {
            
            }
            Handle::Tracker(torrent) => {
            
            }
            _ => unimplemented!(),
        };
        Ok(())
    }

    pub fn add_torrent(&mut self, torrent: Torrent) {
    
    }
}
