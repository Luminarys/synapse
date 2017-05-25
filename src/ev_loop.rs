use std::io;
use mio::{Event, Events, Poll, PollOpt, Ready, Token};
use slab::Slab;
use torrent::{Torrent, tracker};
use torrent::peer::Peer;
use reqwest;
use url::percent_encoding::{percent_encode_byte};
use PEER_ID;
use bencode;

#[derive(Clone, Debug)]
enum Handle {
    Peer { torrent: usize, pid: usize },
    Tracker(usize),
    Incoming(usize),
    Listener,
}

impl Handle {
    pub fn peer(torrent: usize, pid: usize) -> Handle {
        Handle::Peer { torrent: torrent, pid: pid }
    }
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
                    Err(_) => {
                    
                    }
                    _ => (),
                }
            }
        }
        Ok(())
    }

    fn handle_event(&mut self, event: Event) -> io::Result<()> {
        let handle = self.handles.get(event.token()).unwrap().clone();
        match handle {
            Handle::Peer { torrent, pid } => {
                self.handle_peer_ev(torrent, pid, event)?
            }
            Handle::Tracker(torrent) => {
                self.handle_tracker_ev(torrent, event)?
            }
            _ => unimplemented!(),
        };
        Ok(())
    }

    fn handle_peer_ev(&mut self, tid: usize, pid: usize, event: Event) -> io::Result<()> {
        let mut torrent = self.torrents.get_mut(tid).unwrap();
        let mut ready = Ready::readable() | Ready::writable();
        if event.readiness().is_readable() {
            if let Err(e) = torrent.peer_readable(pid) {
                println!("Peer {:?} error'd with {:?}, removing", pid, e);
                torrent.remove_peer(pid);
                return Ok(());
            }
        }
        if event.readiness().is_writable() {
            match torrent.peer_writable(pid) {
                Ok(false) => ready.remove(Ready::writable()),
                Ok(true) => { }
                Err(e) => {
                    println!("Peer {:?} error'd with {:?}, removing", pid, e);
                    torrent.remove_peer(pid);
                    return Ok(());
                }
            }
        }
        self.poll.reregister(&torrent.get_peer_mut(pid).unwrap().conn, event.token(), ready, PollOpt::edge() | PollOpt::oneshot()).unwrap();
        Ok(())
    }

    fn handle_tracker_ev(&mut self, torrent: usize, event: Event) -> io::Result<()> {
        Ok(())
    }

    pub fn add_torrent(&mut self, torrent: Torrent) {
        // TODO: Add the tracker request into the event loop
        let mut url = torrent.info.announce.clone();
        // The fact that I have to do this is genuinely depressing.
        // This will be rewritten as a proper http protocol
        // encoder in the event loop eventually.
        url.push_str("?");
        append_pair(&mut url, "info_hash", &encode_param(&torrent.info.hash));
        append_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
        append_pair(&mut url, "uploaded", "0");
        append_pair(&mut url, "numwant", "20");
        append_pair(&mut url, "downloaded", "0");
        append_pair(&mut url, "left", &torrent.file_size().to_string());
        append_pair(&mut url, "compact", "1");
        append_pair(&mut url, "event", "started");
        append_pair(&mut url, "port", "9999");
        let mut response = if true {
            let mut resp = reqwest::get(&url).unwrap();
            let content = bencode::decode(&mut resp).unwrap();
            tracker::Response::from_bencode(content).unwrap()
        } else {
            tracker::Response::empty()
        };
        let tid = self.torrents.insert(torrent).unwrap();
        let ref mut torrent = self.torrents.get_mut(tid).unwrap();
        response.peers.push("127.0.0.1:8999".parse().unwrap());
        for ip in response.peers.iter() {
            let peer = Peer::new_outgoing(ip, &torrent.info).unwrap();
            let pid = torrent.insert_peer(peer).unwrap();
            let tok = self.handles.insert(Handle::peer(tid, pid)).unwrap();
            self.poll.register(&torrent.get_peer_mut(pid).unwrap().conn, tok, Ready::all(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        }
    }
}

fn append_pair(s: &mut String, k: &str, v: &str) {
    s.push_str(k);
    s.push_str("=");
    s.push_str(v);
    s.push_str("&");
}

fn encode_param(data: &[u8]) -> String {
    let mut resp = String::new();
    for byte in data {
        resp.push_str(percent_encode_byte(*byte));
    }
    resp
}
