use std::io::{self, Read};
use mio::{channel, Event, Events, Poll, PollOpt, Ready, Token};
use mio::tcp::{TcpListener, TcpStream};
use slab::Slab;
use torrent::{Torrent, tracker};
use torrent::peer::Peer;
use reqwest::{self, Url};
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
                    Err(e) => {
                    
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
        let mut ready = Ready::all();
        if event.readiness().is_readable() {
            println!("Peer {:?} readable!", pid);
            if torrent.peer_readable(pid).is_err() {
                println!("Peer {:?} error'd, removing", pid);
                torrent.remove_peer(pid);
                return Ok(());
            }
        }
        if event.readiness().is_writable() {
            println!("Peer {:?} writable!", pid);
            if !torrent.peer_writable(pid)? {
                ready.remove(Ready::writable());
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
        url.push_str("?");
        append_pair(&mut url, "info_hash", &encode_param(&torrent.info.hash));
        append_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
        append_pair(&mut url, "uploaded", "0");
        append_pair(&mut url, "numwant", "5");
        append_pair(&mut url, "downloaded", "0");
        append_pair(&mut url, "left", &torrent.file_size().to_string());
        append_pair(&mut url, "compact", "1");
        append_pair(&mut url, "event", "started");
        append_pair(&mut url, "port", "9999");
        let mut resp = reqwest::get(&url).unwrap();
        let mut content = bencode::decode(&mut resp).unwrap();
        let response = tracker::Response::from_bencode(content).unwrap();
        let tid = self.torrents.insert(torrent).unwrap();
        let ref mut torrent = self.torrents.get_mut(tid).unwrap();
        for ip in response.peers.iter() {
            let peer = Peer::new_outgoing(ip, &torrent.info).unwrap();
            let pid = torrent.insert_peer(peer).unwrap();
            println!("Regiserting peer {:?}", pid);
            let tok = self.handles.insert(Handle::peer(tid, pid)).unwrap();
            self.poll.register(&torrent.get_peer_mut(pid).unwrap().conn, tok, Ready::all(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
        }
        println!("{:?}", response);
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
