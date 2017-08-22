use std::fmt;
use std::io::{self, ErrorKind};
use std::net::{SocketAddrV4, Ipv4Addr, TcpListener};
use amy::{self, Poller, Registrar};
use std::collections::HashMap;
use slog::Logger;
use torrent::peer::PeerConn;
use {handle, CONFIG};

pub struct Listener {
    listener: TcpListener,
    lid: usize,
    incoming: HashMap<usize, PeerConn>,
    poll: Poller,
    reg: Registrar,
    ch: handle::Handle<Request, Message>,
    l: Logger,
}

pub struct Message {
    pub peer: PeerConn,
    pub id: [u8; 20],
    pub hash: [u8; 20],
    pub rsv: [u8; 8],
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "listener msg for torrent: ")?;
        for byte in &self.hash {
            write!(f, "{:X}", byte)?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub enum Request {
    Shutdown,
}

const POLL_INT_MS: usize = 1000;

impl Listener {
    pub fn start(creg: &mut amy::Registrar) -> io::Result<handle::Handle<Message, Request>> {
        let poll = Poller::new()?;
        let mut reg = poll.get_registrar()?;
        let ip = Ipv4Addr::new(0, 0, 0, 0);
        let port = CONFIG.port;
        let listener = TcpListener::bind(SocketAddrV4::new(ip, port))?;
        listener.set_nonblocking(true)?;
        let lid = reg.register(&listener, amy::Event::Both)?;

        let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
        dh.run("listener", move |h, l| {
            Listener {
                listener,
                lid,
                incoming: HashMap::new(),
                poll,
                reg,
                ch: h,
                l,
            }.run()
        });
        Ok(ch)
    }

    pub fn run(&mut self) {
        debug!(self.l, "Accepting connections!");
        while let Ok(res) = self.poll.wait(POLL_INT_MS) {
            for not in res {
                match not.id {
                    id if id == self.lid => self.handle_conn(),
                    id if id == self.ch.rx.get_id() => {
                        if let Ok(Request::Shutdown) = self.ch.recv() {
                            return;
                        }
                    }
                    _ => self.handle_peer(not),
                }
            }
        }
    }

    fn handle_conn(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((conn, _ip)) => {
                    debug!(self.l, "Accepted new connection from {:?}!", _ip);
                    let peer = PeerConn::new_incoming(conn).unwrap();
                    let pid = self.reg.register(peer.sock(), amy::Event::Read).unwrap();
                    self.incoming.insert(pid, peer);
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                _ => {
                    unimplemented!();
                }
            }
        }
    }

    fn handle_peer(&mut self, not: amy::Notification) {
        let pid = not.id;
        match self.incoming.get_mut(&pid).unwrap().readable() {
            Ok(Some(hs)) => {
                debug!(
                    self.l,
                    "Completed handshake({:?}) with peer, transferring!",
                    hs
                );
                let peer = self.incoming.remove(&pid).unwrap();
                self.reg.deregister(peer.sock()).unwrap();
                let hsd = hs.get_handshake_data();
                if self.ch
                    .send(Message {
                        peer,
                        hash: hsd.0,
                        id: hsd.1,
                        rsv: hsd.2,
                    })
                    .is_err()
                {
                    error!(self.l, "failed to send peer to ctrl");
                }
            }
            Ok(None) => {}
            Err(_) => {
                debug!(self.l, "Peer connection failed!");
                self.incoming.remove(&pid);
            }
        }
    }
}
