use std::thread;
use std::io::ErrorKind;
use mio::net::TcpListener;
use mio::{Event, Events, Poll, PollOpt, Ready, Token};
use slab::Slab;
use torrent::Peer;
use {control, CONTROL};

pub struct Listener {
    listener: TcpListener,
    incoming: Slab<Peer, usize>,
    poll: Poll,
}

pub struct Handle { }

impl Handle {
    pub fn dr(&self) { }
}
unsafe impl Sync for Handle {}

const LISTENER: Token = Token(1_000_000);

impl Listener {
    pub fn new() -> Listener {
        let addr = "127.0.0.1:13264".parse().unwrap();
        let listener = TcpListener::bind(&addr).unwrap();
        let poll = Poll::new().unwrap();
        poll.register(&listener, LISTENER, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();

        Listener {
            listener,
            incoming: Slab::with_capacity(256),
            poll
        }
    }

    pub fn run(&mut self) {
        let mut events = Events::with_capacity(128);
        loop {
            self.poll.poll(&mut events, None).unwrap();
            for event in events.iter() {
                match event.token() {
                    LISTENER => self.handle_conn(),
                    _ => self.handle_peer(event),
                }
            }
        }
    }

    fn handle_conn(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((conn, _ip)) => {
                    println!("Accepted new connection from {:?}!", _ip);
                    let peer = Peer::new_incoming(conn).unwrap();
                    let pid = self.incoming.insert(peer).ok().unwrap();
                    let ref mut p = self.incoming.get(pid).unwrap();
                    self.poll.register(&p.conn, Token(pid), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                _ => { unimplemented!(); }
            }
        }
        self.poll.reregister(&self.listener, LISTENER, Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
    }

    fn handle_peer(&mut self, event: Event) {
        let pid = event.token().0;
        let res = self.incoming.get_mut(pid).unwrap().readable().map(|mut msgs| {
            if msgs.len() > 0 {
                Some((msgs.remove(0), msgs))
            } else {
                None
            }
        });
        match res {
            Ok(Some((hs, rest))) => {
                println!("Got HS {:?}, transferring peer!", hs);
                let peer = self.incoming.remove(pid).unwrap();
                self.poll.deregister(&peer.conn).unwrap();
                CONTROL.ctrl_tx.send(control::Request::AddPeer(peer, hs.get_handshake_hash(), rest)).unwrap();
            }
            Ok(None) => {
                let p = self.incoming.get_mut(pid).unwrap();
                self.poll.reregister(&p.conn, Token(pid), Ready::readable(), PollOpt::edge() | PollOpt::oneshot()).unwrap();
            }
            Err(_) => {
                println!("Bad incoming connection, removing!");
                self.incoming.remove(pid);
            }
        }
    }
}

pub fn start() -> Handle {
    thread::spawn(move || {
        Listener::new().run();
    });
    Handle { }
}
