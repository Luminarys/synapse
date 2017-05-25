use std::sync::{mpsc, Arc};
use std::fs::OpenOptions;
use std::thread;
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

pub struct Disk {
    queue: mpsc::Receiver<Request>,
}

pub struct Handle {
    pub tx: mpsc::Sender<Request>,
}

impl Handle {
    pub fn get(&self) -> mpsc::Sender<Request> {
        self.tx.clone()
    }
}

unsafe impl Sync for Handle {}

pub struct Request {
    pub file: PathBuf,
    pub data: Arc<Box<[u8; 16384]>>,
    pub offset: u64,
    pub start: usize,
    pub end: usize,
}

pub struct Response {

}

impl Disk {
    pub fn new(queue: mpsc::Receiver<Request>) -> Disk {
        Disk {
            queue
        }
    }

    pub fn run(&mut self) {
        loop {
            if let Ok(m) = self.queue.recv() {
                OpenOptions::new().write(true).open(&m.file).and_then(|mut f| {
                    f.seek(SeekFrom::Start(m.offset)).unwrap();
                    f.write(&m.data[m.start..m.end])
                }).unwrap();
            } else {
                break;
            }
        }
    }
}

pub fn start() -> Handle {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        Disk::new(rx).run();
    });
    Handle { tx }
}
