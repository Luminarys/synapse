use std::sync::{mpsc, Arc};
use std::fs::OpenOptions;
use std::thread;
use std::io::{Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::PathBuf;

pub struct Disk {
    queue: mpsc::Receiver<Request>,
}

pub struct Handle {
    sender: mpsc::Sender<Request>,
}

impl Handle {
    pub fn get(&self) -> mpsc::Sender<Request> {
        self.sender.clone()
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

impl Disk {
    pub fn new(queue: mpsc::Receiver<Request>) -> Disk {
        Disk {
            queue
        }
    }

    pub fn run(&mut self) {
        loop {
            if let Ok(m) = self.queue.recv() {
                // println!("Got request for file {:?}, at offset {:?}, data from {:?}-{:?}", m.file, m.offset, m.start, m.end);
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
        let mut d = Disk::new(rx);
        d.run();
    });
    Handle { sender: tx }
}
