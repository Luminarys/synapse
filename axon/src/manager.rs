use std::thread;
use std::sync::mpsc;
use mio::{channel, Events, Poll, PollOpt, Ready, Token};
use worker::{Worker, WorkerReq, WorkerResp};
use handle;

pub enum ManagerReq {
    Shutdown,
}

pub enum ManagerResp {
}

pub struct Manager {
    worker_rx: channel::Receiver<WorkerResp>,
    worker_tx: Vec<channel::Sender<WorkerReq>>,
    handle_rx: channel::Receiver<ManagerReq>,
    handle_tx: mpsc::Sender<ManagerResp>,
    event_tx: mpsc::Sender<handle::Event>,
}

impl Manager {
    pub fn new(workers: usize,
               handle_tx: mpsc::Sender<ManagerResp>,
               handle_rx: channel::Receiver<ManagerReq>,
               event_tx: mpsc::Sender<handle::Event>) -> Manager {
        let (tx, rx) = channel::channel();
        let mut chans = Vec::new();
        for _ in 0..workers {
            let (mut w, wtx) = Worker::new(tx.clone());
            chans.push(wtx);
            thread::spawn(move || {
                w.run();
            });
        }

        Manager {
            worker_rx: rx,
            worker_tx: chans,
            handle_tx: handle_tx,
            handle_rx: handle_rx,
            event_tx: event_tx,
        }
    }

    pub fn run(&mut self) {
        const WORKER_TOK: Token = Token(0);
        const HANDLE_TOK: Token = Token(1);
        let poll = Poll::new().unwrap();
        poll.register(&self.worker_rx, WORKER_TOK, Ready::readable(), PollOpt::level());
        poll.register(&self.handle_rx, HANDLE_TOK, Ready::readable(), PollOpt::level());

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
                    _ => {
                        unreachable!();
                    }
                }
            }
        }
    }

    fn process_handle_ev(&mut self) -> bool {
        match self.handle_rx.try_recv() {
            Ok(ManagerReq::Shutdown) => {
                for tx in self.worker_tx.iter() {
                    tx.send(WorkerReq::Shutdown);
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
