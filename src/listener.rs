use std::thread;
use std::io::ErrorKind;
use std::net::TcpListener;
use amy::{self, Poller, Registrar};
use std::collections::HashMap;
use torrent::Peer;
use {control, CONTROL};

pub struct Listener {
    listener: TcpListener,
    lid: usize,
    incoming: HashMap<usize, Peer>,
    poll: Poller,
    reg: Registrar,
    ctrl_tx: amy::Sender<control::Request>,
}

pub struct Handle { }

impl Handle {
    pub fn dr(&self) { }
}
unsafe impl Sync for Handle {}

impl Listener {
    pub fn new() -> Listener {
        let listener = TcpListener::bind("127.0.0.1:13264").unwrap();
        listener.set_nonblocking(true).unwrap();
        let poll = Poller::new().unwrap();
        let reg = poll.get_registrar().unwrap();
        let lid = reg.register(&listener, amy::Event::Both).unwrap();

        Listener {
            listener,
            lid,
            incoming: HashMap::new(),
            poll,
            reg,
            ctrl_tx: CONTROL.ctrl_tx(),
        }
    }

    pub fn run(&mut self) {
        loop {
            for not in self.poll.wait(15).unwrap() {
                match not.id {
                    id if id == self.lid => self.handle_conn(),
                    _ => self.handle_peer(not),
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
                    let pid = self.reg.register(&peer.conn, amy::Event::Read).unwrap();
                    self.incoming.insert(pid, peer);
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                _ => { unimplemented!(); }
            }
        }
    }

    fn handle_peer(&mut self, not: amy::Notification) {
        let pid = not.id;
        let res = self.incoming.get_mut(&pid).unwrap().readable().map(|mut msgs| {
            if msgs.len() > 0 {
                Some((msgs.remove(0), msgs))
            } else {
                None
            }
        });
        match res {
            Ok(Some((hs, rest))) => {
                println!("Got HS {:?}, transferring peer!", hs);
                let peer = self.incoming.remove(&pid).unwrap();
                self.reg.deregister(peer.conn.fd()).unwrap();
                self.ctrl_tx.send(control::Request::AddPeer(peer, hs.get_handshake_hash(), rest)).unwrap();
            }
            Ok(_) => { println!("Reading HS"); }
            Err(_) => {
                println!("Bad incoming connection, removing!");
                self.incoming.remove(&pid);
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
