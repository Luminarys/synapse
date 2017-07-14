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

pub struct ACChans {
    pub disk_tx: amy::Sender<disk::Request>,
    pub disk_rx: amy::Receiver<disk::Response>,

    pub rpc_tx: amy::Sender<rpc::CMessage>,
    pub rpc_rx: amy::Receiver<rpc::Request>,

    pub trk_tx: amy::Sender<tracker::Request>,
    pub trk_rx: amy::Receiver<tracker::Response>,

    pub lst_tx: amy::Sender<listener::Request>,
    pub lst_rx: amy::Receiver<listener::Message>,
}

struct ACIOData {
    poll: amy::Poller,
    reg: amy::Registrar,
    peers: HashMap<usize, torrent::PeerConn>,
    events: Vec<cio::Event>,
    l: Logger,
    chans: ACChans,
}

impl ACIO {
    pub fn new(poll: amy::Poller, reg: amy::Registrar, chans: ACChans, l: Logger) -> Result<ACIO> {
        let data = ACIOData {
            poll,
            reg,
            chans,
            l,
            peers: HashMap::new(),
            events: Vec::new(),
        };
        Ok(ACIO {
            data: Rc::new(UnsafeCell::new(data)),
        })
    }

    fn process_event(&mut self, not: amy::Notification, events: &mut Vec<cio::Event>) {
        let id = not.id;
        if self.d().chans.disk_rx.get_id() == id {
            while let Ok(t) = self.d().chans.disk_rx.try_recv() {
                events.push(cio::Event::Disk(Ok(t)));
            }
        } else if self.d().chans.rpc_rx.get_id() == id {
            while let Ok(t) = self.d().chans.rpc_rx.try_recv() {
                events.push(cio::Event::RPC(Ok(t)));
            }
        } else if self.d().chans.trk_rx.get_id() == id {
            while let Ok(t) = self.d().chans.trk_rx.try_recv() {
                events.push(cio::Event::Tracker(Ok(t)));
            }
        } else if self.d().chans.lst_rx.get_id() == id {
            while let Ok(t) = self.d().chans.lst_rx.try_recv() {
                events.push(cio::Event::Listener(Ok(t)));
            }
        } else if self.d().peers.contains_key(&id) {
            if let Err(e) = self.process_peer_ev(not, events) {
                self.d().remove_peer(id);
                events.push(cio::Event::Peer { peer: id, event: Err(e) });
            }
        } else {
            // Timer event
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

    fn add_peer(&mut self, mut peer: torrent::PeerConn) -> Result<cio::PID> {
       let id = self.d().reg.register(peer.sock(), amy::Event::Both)
            .chain_err(|| ErrorKind::IO)?;
       peer.sock_mut().throttle.as_mut().map(|t| t.id = id);
       self.d().peers.insert(id, peer);
       Ok(id)
    }

    fn remove_peer(&mut self, peer: cio::PID) {
        self.d().remove_peer(peer);
    }

    fn flush_peers(&mut self, peers: Vec<cio::PID>) {
        let mut events = Vec::new();

        for peer in peers {
            let not = amy::Notification { id: peer, event: amy::Event::Both };
            if let Err(e) = self.process_peer_ev(not, &mut events) {
                self.d().remove_peer(peer);
                events.push(cio::Event::Peer { peer, event: Err(e) });
            }
        }

        self.d().events.extend(events.drain(..));
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

    fn msg_rpc(&mut self, msg: rpc::CMessage) {
        if self.d().chans.rpc_tx.send(msg).is_err() {
            self.d().events.push(
                cio::Event::RPC(Err(ErrorKind::Channel("Couldn't send to RPC chan").into()))
            );
        }
    }

    fn msg_trk(&mut self, msg: tracker::Request) {
        if self.d().chans.trk_tx.send(msg).is_err() {
            self.d().events.push(
                cio::Event::Tracker(Err(ErrorKind::Channel("Couldn't send to trk chan").into()))
            );
        }
    }

    fn msg_disk(&mut self, msg: disk::Request) {
        if self.d().chans.disk_tx.send(msg).is_err() {
            self.d().events.push(
                cio::Event::Disk(Err(ErrorKind::Channel("Couldn't send to disk chan").into()))
            );
        }
    }

    fn msg_listener(&mut self, msg: listener::Request) {
        if self.d().chans.lst_tx.send(msg).is_err() {
            self.d().events.push(
                cio::Event::Listener(Err(ErrorKind::Channel("Couldn't send to disk chan").into()))
            );
        }
    }

    fn set_timer(&mut self, interval: usize) -> Result<cio::TID> {
        self.d().reg.set_interval(interval)
            .chain_err(|| ErrorKind::IO)
    }

    fn new_handle(&self) -> Self {
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
