use std::sync::mpsc;
use std::fs::OpenOptions;
use std::{fmt, thread};
use std::io::{Seek, SeekFrom, Write, Read};
use std::path::PathBuf;
use {amy, CONTROL};

pub struct Disk {
    queue: mpsc::Receiver<Request>,
    disk_tx: amy::Sender<Response>,
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

pub enum Request {
    Write { data: Box<[u8; 16384]>, locations: Vec<Location> },
    Read { data: Box<[u8; 16384]>, locations: Vec<Location>, context: Ctx }
}

pub struct Ctx {
    pub id: usize,
    pub idx: u32,
    pub begin: u32,
    pub length: u32,
}

impl Ctx {
    pub fn new(id: usize, idx: u32, begin: u32, length: u32) -> Ctx {
        Ctx { id, idx, begin, length }
    }
}

impl Request {
    pub fn write(data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Write { data, locations }
    }

    pub fn read(context: Ctx, data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Read { context, data, locations }
    }
}

pub struct Location {
    pub file: PathBuf,
    pub offset: u64,
    pub start: usize,
    pub end: usize,
}

impl Location {
    pub fn new(file: PathBuf, offset: u64, start: usize, end: usize) -> Location {
        Location { file, offset, start, end }
    }
}

pub struct Response {
    pub context: Ctx,
    pub data: Box<[u8; 16384]>,
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "disk::Response")
    }
}

impl Disk {
    pub fn new(queue: mpsc::Receiver<Request>) -> Disk {
        Disk {
            queue,
            disk_tx: CONTROL.disk_tx(),
        }
    }

    pub fn run(&mut self) {
        loop {
            match self.queue.recv() {
                Ok(Request::Write { data, locations } ) => {
                    for loc in locations {
                        OpenOptions::new().write(true).open(&loc.file).and_then(|mut f| {
                            f.seek(SeekFrom::Start(loc.offset)).unwrap();
                            f.write(&data[loc.start..loc.end])
                        }).unwrap();
                    }
                }
                Ok(Request::Read { context, mut data, locations } ) =>  {
                    for loc in locations {
                        OpenOptions::new().read(true).open(&loc.file).and_then(|mut f| {
                            f.seek(SeekFrom::Start(loc.offset)).unwrap();
                            f.read(&mut data[loc.start..loc.end])
                        }).unwrap();
                    }
                    self.disk_tx.send(Response { context, data }).unwrap();
                }
                _ => break,
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
