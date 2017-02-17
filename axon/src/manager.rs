use std::{thread, io};
use std::sync::mpsc;
use std::collections::HashSet;
use mio::{channel, Events, Poll, PollOpt, Ready, Token};
use mio::tcp::{TcpListener};
use slab::Slab;
use worker::{Worker, WorkerReq, WorkerResp};
use peer::IncomingPeer;
use message::Message;
use handle;

pub enum ManagerReq {
    Shutdown,
}

pub enum ManagerResp {
}

struct WorkerData {
    tx: channel::Sender<WorkerReq>,
    torrents: HashSet<[u8; 20]>,
}

impl WorkerData {
    fn new(tx: channel::Sender<WorkerReq>) -> WorkerData {
        WorkerData {
            tx: tx,
            torrents: HashSet::new(),
        }
    }
}

pub struct Manager {
    worker_rx: channel::Receiver<WorkerResp>,
    workers: Vec<WorkerData>,
    handle_rx: channel::Receiver<ManagerReq>,
    handle_tx: mpsc::Sender<ManagerResp>,
    event_tx: mpsc::Sender<handle::Event>,
    listener: TcpListener,
    incoming_conns: Slab<IncomingPeer, Token>,
}

impl Manager {
    pub fn new(worker_num: usize,
               handle_tx: mpsc::Sender<ManagerResp>,
               handle_rx: channel::Receiver<ManagerReq>,
               event_tx: mpsc::Sender<handle::Event>) -> Manager {
        let (tx, rx) = channel::channel();
        let mut workers = Vec::new();
        for _ in 0..worker_num {
            let (mut w, wtx) = Worker::new(tx.clone());
            workers.push(WorkerData::new(wtx));
            thread::spawn(move || {
                w.run();
            });
        }

        Manager {
            worker_rx: rx,
            workers: workers,
            handle_tx: handle_tx,
            handle_rx: handle_rx,
            event_tx: event_tx,
            listener: TcpListener::bind(&"127.0.0.1:42069".parse().unwrap()).unwrap(),
            incoming_conns: Slab::with_capacity(128),
        }
    }

    pub fn run(&mut self) -> io::Result<()> {
        const WORKER_TOK: Token = Token(1000);
        const HANDLE_TOK: Token = Token(1001);
        const LISTENER_TOK: Token = Token(1002);

        let poll = Poll::new()?;
        poll.register(&self.worker_rx, WORKER_TOK, Ready::readable(), PollOpt::level())?;
        poll.register(&self.handle_rx, HANDLE_TOK, Ready::readable(), PollOpt::level())?;
        poll.register(&self.listener, LISTENER_TOK, Ready::readable(), PollOpt::edge())?;

        let mut events = Events::with_capacity(1024);
        loop {
            poll.poll(&mut events, None).unwrap();
            
            for event in events.iter() {
                match event.token() {
                    WORKER_TOK => {
                        self.process_worker_ev();
                    }
                    HANDLE_TOK => {
                        if self.process_handle_ev() {
                            self.cleanup();
                            break;
                        }
                    }
                    LISTENER_TOK => {
                        let (s, _) = self.listener.accept()?;
                        if self.incoming_conns.has_available() {
                            let vi = self.incoming_conns.vacant_entry().unwrap();
                            poll.register(&s, vi.index(), Ready::readable(), PollOpt::edge())?;
                            let p = IncomingPeer::new(s);
                            vi.insert(p);
                        }
                        poll.reregister(&self.listener, LISTENER_TOK, Ready::readable(), PollOpt::edge())?;

                    }
                    t => {
                        let v = {
                            let mut peer = self.incoming_conns.get_mut(t).unwrap();
                            peer.readable()?
                        };
                        if let Some(msg) = v {
                            match msg {
                                Message::Handshake { rsv, hash, id } => {
                                    let peer = self.incoming_conns.remove(t).unwrap();
                                    let w = self.get_available_worker(&hash);
                                    w.tx.send(WorkerReq::NewConn{ id: id, hash: hash, peer: peer });
                                }
                                _ => {
                                    unimplemented!();
                                }
                            }
                        } else {
                            let mut peer = self.incoming_conns.get_mut(t).unwrap();
                            poll.reregister(peer.socket(), t, Ready::readable(), PollOpt::edge())?;
                        }
                    }
                }
            }
        }
    }

    fn get_available_worker(&self, hash: &[u8; 20]) -> &WorkerData {
        self.workers.iter()
            .find(|w| w.torrents.contains(hash))
            .unwrap_or_else(|| {
                self.workers.iter().min_by_key(|w| {
                    w.torrents.len()
                }).unwrap()
            })
    }

    fn process_handle_ev(&mut self) -> bool {
        match self.handle_rx.try_recv() {
            Ok(ManagerReq::Shutdown) => {
                for w in self.workers.iter() {
                    w.tx.send(WorkerReq::Shutdown);
                }
                return true;
            }
            _ => {
            
            }
        };
        false
    }

    fn process_worker_ev(&mut self) {
        match self.worker_rx.try_recv() {
            _ => {
            
            }
        }
    
    }

    fn cleanup(&mut self) {
    
    }
}
