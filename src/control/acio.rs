use std::collections::HashMap;
use std::cell::UnsafeCell;
use std::rc::Rc;
use slog::Logger;

use amy;

use {rpc, tracker, disk, listener, torrent};
use control::cio::{self, Result, ResultExt, ErrorKind};

const POLL_INT_MS: usize = 3;

/// Amy based CIO implementation. Currently the default one used.
pub struct ACIO {
    data: Rc<UnsafeCell<ACIOData>>,
}

struct ACIOData {
    poll: amy::Poller,
    reg: amy::Registrar,
    peers: HashMap<usize, torrent::PeerConn>,
    events: Vec<cio::Event>,
    l: Logger,
}

impl ACIO {
    pub fn new(l: Logger) -> Result<ACIO> {
        let poll = amy::Poller::new().chain_err(|| ErrorKind::IO)?;
        let reg = poll.get_registrar().chain_err(|| ErrorKind::IO)?;
        let data = ACIOData {
            poll,
            reg,
            peers: HashMap::new(),
            events: Vec::new(),
            l
        };
        Ok(ACIO {
            data: Rc::new(UnsafeCell::new(data)),
        })
    }

    fn process_event(&mut self, not: amy::Notification, events: &mut Vec<cio::Event>) {
        let id = not.id;
        if self.d().peers.contains_key(&id) {
            if let Err(e) = self.process_peer_ev(not, events) {
                self.d().remove_peer(id);
                events.push(cio::Event::Peer { peer: id, event: Err(e) });
            }
        } else {
            events.push(cio::Event::Timer(id));
        }
    }

    fn process_peer_ev(&mut self, not: amy::Notification, events: &mut Vec<cio::Event>) -> Result<()> {
        let d = self.d();
        let peer = d.peers.get_mut(&not.id).unwrap();
        let ev = not.event;
        if ev.readable() {
            while let Some(msg) = peer.readable().chain_err(|| ErrorKind::IO)? {
                events.push(cio::Event::Peer { peer: not.id, event: Ok(msg) });
            }
        }
        if ev.writable() {
            peer.writable().chain_err(|| ErrorKind::IO)?;
        }
        Ok(())
    }

    fn d(&self) -> &'static mut ACIOData {
        unsafe {
            self.data.get().as_mut().unwrap()
        }
    }
}

//pub enum Event {
//    Timer(cio::TID),
//    Peer { peer: cio::PID, event: Result<torrent::Message> },
//    RPC(Result<rpc::Request>),
//    Tracker(Result<tracker::Response>),
//    Disk(Result<disk::Response>),
//    Listener(Result<listener::Message>),
//}

impl cio::CIO for ACIO {
    fn poll(&mut self, events: &mut Vec<cio::Event>) {
        match self.d().poll.wait(POLL_INT_MS) {
            Ok(evs) => {
                for event in evs {
                    self.process_event(event, events);
                }
            }
            Err(e) => {
                warn!(self.d().l, "Failed to poll for events: {:?}", e);
            }
        }
        for event in self.d().events.drain(..) {
            events.push(event);
        }
    }

    fn add_peer(&mut self, peer: torrent::PeerConn) -> Result<cio::PID> {
        self.d().reg.register(peer.sock(), amy::Event::Both)
            .chain_err(|| ErrorKind::IO)
    }

    fn msg_peer(&mut self, pid: cio::PID, msg: torrent::Message) {
        let d = self.d();
        let err = if let Some(peer) = d.peers.get_mut(&pid) {
            peer.write_message(msg).chain_err(|| ErrorKind::IO).err()
        } else {
            // might happen if removed but still present in a torrent
            debug!(d.l, "Tried to message peer which has been removed!");
            None
        };
        if let Some(e) = err {
            d.remove_peer(pid);
            d.events.push(cio::Event::Peer { peer: pid, event: Err(e) });
        }
    }

    fn msg_rpc(&mut self, msg: rpc::Message) {
    }

    fn msg_trk(&mut self, msg: tracker::Request) {
    }

    fn msg_disk(&mut self, msg: disk::Request) {
    }

    fn msg_listener(&mut self, msg: listener::Request) {
    }

    fn set_timer(&mut self, interval: usize) -> Result<cio::TID> {
        self.d().reg.set_interval(interval)
            .chain_err(|| ErrorKind::IO)
    }

    fn handle(&self) -> Self {
        ACIO {
            data: self.data.clone(),
        }
    }
}

impl ACIOData {
    fn remove_peer(&mut self, pid: cio::PID) {
        if let Some(p) = self.peers.remove(&pid) {
            while let Err(e) = self.reg.deregister(p.sock()) {
                warn!(self.l, "Failed to deregister sock: {:?}", e);
            }
        }
    }
}
