use std::cell::RefCell;
use std::rc::Rc;
use std::time;

use amy;

use {disk, listener, rpc, torrent, tracker};
use CONFIG;
use control::cio::{self, ErrorKind, Result, ResultExt};
use util::UHashMap;

const POLL_INT_MS: usize = 1000;
const PRUNE_GOAL: usize = 50;

/// Amy based CIO implementation. Currently the default one used.
pub struct ACIO {
    data: Rc<RefCell<ACIOData>>,
}

pub struct ACChans {
    pub disk_tx: amy::Sender<disk::Request>,
    pub disk_rx: amy::Receiver<disk::Response>,

    pub rpc_tx: amy::Sender<rpc::CtlMessage>,
    pub rpc_rx: amy::Receiver<rpc::Message>,

    pub trk_tx: amy::Sender<tracker::Request>,
    pub trk_rx: amy::Receiver<tracker::Response>,

    pub lst_tx: amy::Sender<listener::Request>,
    pub lst_rx: amy::Receiver<listener::Message>,
}

struct ACIOData {
    poll: amy::Poller,
    reg: amy::Registrar,
    peers: UHashMap<torrent::PeerConn>,
    events: Vec<cio::Event>,
    chans: ACChans,
}

impl ACIO {
    pub fn new(poll: amy::Poller, reg: amy::Registrar, chans: ACChans) -> ACIO {
        let data = ACIOData {
            poll,
            reg,
            chans,
            peers: UHashMap::default(),
            events: Vec::new(),
        };
        ACIO {
            data: Rc::new(RefCell::new(data)),
        }
    }

    fn process_event(&self, not: amy::Notification, events: &mut Vec<cio::Event>) {
        let id = not.id;

        let mut d = self.data.borrow_mut();

        if d.chans.disk_rx.get_id() == id {
            while let Ok(t) = d.chans.disk_rx.try_recv() {
                events.push(cio::Event::Disk(Ok(t)));
            }
        } else if d.chans.rpc_rx.get_id() == id {
            while let Ok(t) = d.chans.rpc_rx.try_recv() {
                events.push(cio::Event::RPC(Ok(t)));
            }
        } else if d.chans.trk_rx.get_id() == id {
            while let Ok(t) = d.chans.trk_rx.try_recv() {
                events.push(cio::Event::Tracker(Ok(t)));
            }
        } else if d.chans.lst_rx.get_id() == id {
            while let Ok(t) = d.chans.lst_rx.try_recv() {
                events.push(cio::Event::Listener(Ok(Box::new(t))));
            }
        } else if d.peers.contains_key(&id) {
            if let Err(e) = self.process_peer_ev(not, events, &mut d.peers) {
                d.remove_peer(id);
                events.push(cio::Event::Peer {
                    peer: id,
                    event: Err(e),
                });
            }
        } else {
            // Timer event
            events.push(cio::Event::Timer(id));
        }
    }

    fn process_peer_ev(
        &self,
        not: amy::Notification,
        events: &mut Vec<cio::Event>,
        peers: &mut UHashMap<torrent::PeerConn>,
    ) -> Result<()> {
        if let Some(peer) = peers.get_mut(&not.id) {
            let ev = not.event;
            if ev.readable() {
                while let Some(msg) = peer.readable().chain_err(|| ErrorKind::IO)? {
                    events.push(cio::Event::Peer {
                        peer: not.id,
                        event: Ok(msg),
                    });
                }
            }
            if ev.writable() {
                peer.writable().chain_err(|| ErrorKind::IO)?;
            }
        }
        Ok(())
    }
}

impl cio::CIO for ACIO {
    fn poll(&mut self, events: &mut Vec<cio::Event>) {
        {
            let mut d = self.data.borrow_mut();

            for event in d.events.drain(..) {
                events.push(event);
            }
        }

        let res = self.data.borrow_mut().poll.wait(POLL_INT_MS);

        match res {
            Ok(evs) => for event in evs {
                self.process_event(event, events);
            },
            Err(e) => {
                error!("Failed to poll for events: {:?}", e);
            }
        }
    }

    fn add_peer(&mut self, mut peer: torrent::PeerConn) -> Result<cio::PID> {
        if self.data.borrow().peers.len() > CONFIG.net.max_open_sockets {
            let mut pruned = Vec::new();
            for (id, peer) in &self.data.borrow().peers {
                if peer.last_action().elapsed()
                    > time::Duration::from_secs(CONFIG.peer.prune_timeout)
                {
                    pruned.push(*id)
                }
                if pruned.len() == PRUNE_GOAL {
                    break;
                }
            }
            // We couldn't even prune anything, this client must be really busy...
            // Either way just return an error
            if pruned.is_empty() {
                return Err(ErrorKind::Full.into());
            }

            for id in pruned {
                self.remove_peer(id);
            }
        }
        let id = self.data.borrow_mut()
            .reg
            .register(peer.sock(), amy::Event::Both)
            .chain_err(|| ErrorKind::IO)?;
        peer.sock_mut().throttle.as_mut().map(|t| t.id = id);
        self.data.borrow_mut().peers.insert(id, peer);
        Ok(id)
    }

    fn get_peer<T, F: FnOnce(&mut torrent::PeerConn) -> T>(
        &mut self,
        pid: cio::PID,
        f: F,
    ) -> Option<T> {
        if let Some(p) = self.data.borrow_mut().peers.get_mut(&pid) {
            Some(f(p))
        } else {
            None
        }
    }

    fn remove_peer(&self, peer: cio::PID) {
        self.data.borrow_mut().remove_peer(peer);
    }

    fn flush_peers(&mut self, peers: Vec<cio::PID>) {
        let mut events = Vec::new();
        let mut d = self.data.borrow_mut();

        for peer in peers {
            let not = amy::Notification {
                id: peer,
                event: amy::Event::Both,
            };
            if let Err(e) = self.process_peer_ev(not, &mut events, &mut d.peers) {
                d.remove_peer(peer);
                events.push(cio::Event::Peer {
                    peer,
                    event: Err(e),
                });
            }
        }

        d.events.extend(events.drain(..));
    }

    fn msg_peer(&mut self, pid: cio::PID, msg: torrent::Message) {
        let mut d = self.data.borrow_mut();
        let err = if let Some(peer) = d.peers.get_mut(&pid) {
            peer.write_message(msg).chain_err(|| ErrorKind::IO).err()
        } else {
            // might happen if removed but still present in a torrent
            debug!("Tried to message peer which has been removed!");
            None
        };
        if let Some(e) = err {
            d.remove_peer(pid);
            d.events.push(cio::Event::Peer {
                peer: pid,
                event: Err(e),
            });
        }
    }

    fn msg_rpc(&mut self, msg: rpc::CtlMessage) {
        let mut d = self.data.borrow_mut();

        if d.chans.rpc_tx.send(msg).is_err() {
            d.events.push(cio::Event::RPC(Err(ErrorKind::Channel(
                "Couldn't send to RPC chan",
            ).into())));
        }
    }

    fn msg_trk(&mut self, msg: tracker::Request) {
        let mut d = self.data.borrow_mut();

        if d.chans.trk_tx.send(msg).is_err() {
            d
                .events
                .push(cio::Event::Tracker(Err(ErrorKind::Channel(
                    "Couldn't send to trk chan",
                ).into())));
        }
    }

    fn msg_disk(&mut self, msg: disk::Request) {
        let mut d = self.data.borrow_mut();

        if d.chans.disk_tx.send(msg).is_err() {
            d
                .events
                .push(cio::Event::Disk(Err(ErrorKind::Channel(
                    "Couldn't send to disk chan",
                ).into())));
        }
    }

    fn msg_listener(&mut self, msg: listener::Request) {
        let mut d = self.data.borrow_mut();

        if d.chans.lst_tx.send(msg).is_err() {
            d
                .events
                .push(cio::Event::Listener(Err(ErrorKind::Channel(
                    "Couldn't send to disk chan",
                ).into())));
        }
    }

    fn set_timer(&mut self, interval: usize) -> Result<cio::TID> {
        self.data.borrow_mut()
            .reg
            .set_interval(interval)
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
            if let Err(e) = self.reg.deregister(p.sock()) {
                error!("Failed to deregister sock: {:?}", e);
            }
            self.events.push(cio::Event::Peer {
                peer: pid,
                event: Err(ErrorKind::Request.into()),
            });
        }
    }
}
