use std::io::{self, ErrorKind};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, TcpStream};
use std::{fmt, thread};

use amy::{self, Poller};

use {handle, CONFIG};

pub struct Listener {
    listener: TcpListener,
    lid: usize,
    poll: Poller,
    ch: handle::Handle<Request, Message>,
}

pub struct Message {
    pub conn: TcpStream,
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "listener msg")?;
        Ok(())
    }
}

#[derive(Debug)]
pub enum Request {
    Ping,
    Shutdown,
}

const POLL_INT_MS: usize = 1000;

impl Listener {
    pub fn start(
        creg: &mut amy::Registrar,
    ) -> io::Result<(handle::Handle<Message, Request>, thread::JoinHandle<()>)> {
        let poll = Poller::new()?;
        let mut reg = poll.get_registrar();
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
                poll,
                ch: h,
            }
            .run()
        })?;
        Ok((ch, th))
    }

    pub fn run(&mut self) {
        debug!("Accepting connections!");
        loop {
            match self.poll.wait(POLL_INT_MS) {
                Ok(res) => {
                    for not in res {
                        match not.id {
                            id if id == self.lid => self.handle_conn(),
                            id if id == self.ch.rx.get_id() => loop {
                                match self.ch.recv() {
                                    Ok(Request::Ping) => continue,
                                    Ok(Request::Shutdown) => return,
                                    _ => break,
                                }
                            },
                            _ => unreachable!(),
                        }
                    }
                }
                Err(e) => error!("Failed to poll for events: {}", e),
            }
        }
    }

    fn handle_conn(&mut self) {
        loop {
            match self.listener.accept() {
                Ok((conn, ip)) => {
                    debug!("Accepted new connection from {:?}!", ip);
                    if conn.set_nonblocking(true).is_err() {
                        continue;
                    }
                    if self.ch.send(Message { conn: conn }).is_err() {
                        error!("failed to send peer to ctrl");
                    }
                }
                Err(ref e) if e.kind() == ErrorKind::WouldBlock => {
                    break;
                }
                Err(e) => {
                    error!("Unexpected error occured during accept: {}!", e);
                }
            }
        }
    }
}
