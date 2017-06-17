use std::thread;
use std::io::ErrorKind;
use std::net::{SocketAddrV4, Ipv4Addr, TcpListener};
use amy::{self, Poller, Registrar};
use std::collections::HashMap;
use torrent::Peer;
use {control, CONTROL, CONFIG, TC};

pub struct Listener {
    listener: TcpListener,
    lid: usize,
    incoming: HashMap<usize, Peer>,
    poll: Poller,
    reg: Registrar,
}

pub struct Handle { }

impl Handle {
    pub fn init(&self) { }
}

unsafe impl Sync for Handle {}

impl Listener {
    pub fn new() -> Listener {
        let ip = Ipv4Addr::new(0, 0, 0, 0);
        let port = CONFIG.get().port;
        let listener = TcpListener::bind(SocketAddrV4::new(ip, port)).unwrap();
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
        }
    }

    pub fn run(&mut self) {
        loop {
            let res = if let Ok(r) = self.poll.wait(15) { r } else { break; };
            for not in res {
                match not.id {
                    id if id == self.lid => self.handle_conn(),
                    _ => self.handle_peer(not),
                }
            }
        }
        println!("Listener shutdown");
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
        let res = self.incoming.get_mut(&pid).unwrap().read();
        match res {
            Ok(Some(hs)) => {
                println!("Got HS {:?}, transferring peer!", hs);
                let peer = self.incoming.remove(&pid).unwrap();
                self.reg.deregister(&peer.conn).unwrap();
                CONTROL.ctrl_tx.lock().unwrap().send(control::Request::AddPeer(peer, hs.get_handshake_hash())).unwrap();
            }
            Ok(_) => { }
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
        use std::sync::atomic;
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
    });
    Handle { }
}
