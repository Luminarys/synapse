use std::{fmt, thread};
use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};

use amy::{self, Poller, Registrar};

use torrent::peer::reader::Reader;
use {handle, CONFIG};
use util::UHashMap;

pub struct Listener {
    listener: TcpListener,
    lid: usize,
    incoming: UHashMap<(TcpStream, Reader)>,
    poll: Poller,
    reg: Registrar,
    ch: handle::Handle<Request, Message>,
}

pub struct Message {
    pub conn: TcpStream,
    pub reader: Reader,
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
    pub fn start(
        creg: &mut amy::Registrar,
    ) -> io::Result<(handle::Handle<Message, Request>, thread::JoinHandle<()>)> {
        let poll = Poller::new()?;
        let mut reg = poll.get_registrar()?;
        let ip = Ipv4Addr::new(0, 0, 0, 0);
        let port = CONFIG.port;
        let listener = TcpListener::bind(SocketAddrV4::new(ip, port))?;
        listener.set_nonblocking(true)?;
        let lid = reg.register(&listener, amy::Event::Both)?;

        let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
        let th = dh.run("listener", move |h| {
            Listener {
                listener,
                lid,
                incoming: UHashMap::default(),
                poll,
                reg,
                ch: h,
            }.run()
        })?;
        Ok((ch, th))
    }

    pub fn run(&mut self) {
        debug!("Accepting connections!");
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
                Ok((conn, ip)) => {
                    debug!("Accepted new connection from {:?}!", ip);
                    let pid = self.reg.register(&conn, amy::Event::Read).unwrap();
                    self.incoming.insert(pid, (conn, Reader::new()));
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

        let res = {
            let &mut (ref mut conn, ref mut reader) = self.incoming.get_mut(&pid).unwrap();
            reader.readable(conn)
        };

        match res {
            Ok(Some(hs)) => {
                debug!("Completed handshake({:?}) with peer, transferring!", hs);
                let (conn, reader) = self.incoming.remove(&pid).unwrap();
                self.reg.deregister(&conn).unwrap();
                let hsd = hs.get_handshake_data();
                if self.ch
                    .send(Message {
                        conn,
                        reader,
                        hash: hsd.0,
                        id: hsd.1,
                        rsv: hsd.2,
                    })
                    .is_err()
                {
                    error!("failed to send peer to ctrl");
                }
            }
            Ok(None) => {}
            Err(_) => {
                debug!("Peer connection failed!");
                self.incoming.remove(&pid);
            }
        }
    }
}
