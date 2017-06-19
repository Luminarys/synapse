use std::sync::{mpsc, Arc, atomic};
use std::{fs, fmt, thread, path};
use std::io::{Seek, SeekFrom, Write, Read};
use std::fmt::Write as FWrite;
use std::path::PathBuf;
use {CONTROL, CONFIG, TC};

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

pub enum Request {
    Write { data: Box<[u8; 16384]>, locations: Vec<Location> },
    Read { data: Box<[u8; 16384]>, locations: Vec<Location>, context: Ctx },
    Serialize { data: Vec<u8>, hash: [u8; 20] },
    Delete { hash: [u8; 20] },
    Shutdown,
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

    pub fn serialize(data: Vec<u8>, hash: [u8; 20]) -> Request {
        Request::Serialize { data, hash }
    }

    pub fn delete(hash: [u8; 20]) -> Request {
        Request::Delete { hash }
    }

    pub fn shutdown() -> Request {
        Request::Shutdown
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
    pub data: Arc<Box<[u8; 16384]>>,
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
        }
    }

    pub fn run(&mut self) {
        let ref sd = CONFIG.get().session;
        fs::create_dir_all(sd).unwrap();
        loop {
            match self.queue.recv() {
                Ok(Request::Write { data, locations }) => {
                    for loc in locations {
                        fs::OpenOptions::new().write(true).open(&loc.file).and_then(|mut f| {
                            f.seek(SeekFrom::Start(loc.offset)).unwrap();
                            f.write(&data[loc.start..loc.end])
                        }).unwrap();
                    }
                }
                Ok(Request::Read { context, mut data, locations }) =>  {
                    for loc in locations {
                        fs::OpenOptions::new().read(true).open(&loc.file).and_then(|mut f| {
                            f.seek(SeekFrom::Start(loc.offset)).unwrap();
                            f.read(&mut data[loc.start..loc.end])
                        }).unwrap();
                    }
                    let data = Arc::new(data);
                    CONTROL.disk_tx.lock().unwrap().send(Response { context, data }).unwrap();
                }
                Ok(Request::Serialize { data, hash }) => {
                    let mut pb = path::PathBuf::from(sd);
                    pb.push(torrent_name(&hash));
                    let res = fs::OpenOptions::new().write(true).create(true).open(&pb).and_then(|mut f| {
                        f.write(&data)
                    });
                    if let Err(e) = res {
                        println!("Failed to serialize torrent {:?}!", e);
                    }
                }
                Ok(Request::Delete { hash } ) => {
                    let mut pb = path::PathBuf::from(sd);
                    pb.push(torrent_name(&hash));
                    fs::remove_file(pb).unwrap();
                }
                Ok(Request::Shutdown) => {
                    println!("Disk thread shutting down!");
                    break
                }
                _ => break,
            }
        }
    }
}

fn torrent_name(hash: &[u8; 20]) -> String {
    let mut hash_str = String::new();
    for i in 0..20 {
        write!(&mut hash_str, "{:X}", hash[i]).unwrap();
    }
    hash_str
}

pub fn start() -> Handle {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        Disk::new(rx).run();
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
    });
    Handle { tx }
}
