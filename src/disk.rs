use std::sync::{mpsc, Arc, atomic};
use std::{fs, fmt, thread, path};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;
use torrent::Info;
use slog::Logger;
use util::torrent_name;
use threadpool::ThreadPool;
use {CONTROL, CONFIG, TC};

pub struct Disk {
    queue: mpsc::Receiver<Request>,
    l: Logger,
    pool: ThreadPool,
    threads: usize,
}

pub struct Handle {
    pub tx: mpsc::Sender<Request>,
}

impl Handle {
    pub fn init(&self) { }

    pub fn get(&self) -> mpsc::Sender<Request> {
        self.tx.clone()
    }
}

unsafe impl Sync for Handle {}

pub enum Request {
    Write { tid: usize, data: Box<[u8; 16384]>, locations: Vec<Location> },
    Read { data: Box<[u8; 16384]>, locations: Vec<Location>, context: Ctx },
    Serialize { tid: usize, data: Vec<u8>, hash: [u8; 20] },
    Delete { tid: usize, hash: [u8; 20] },
    Validate { tid: usize, info: Arc<Info> },
    Shutdown,
}

pub struct Ctx {
    pub pid: usize,
    pub tid: usize,
    pub idx: u32,
    pub begin: u32,
    pub length: u32,
}

impl Ctx {
    pub fn new(pid: usize, tid: usize, idx: u32, begin: u32, length: u32) -> Ctx {
        Ctx { pid, tid, idx, begin, length }
    }
}

impl Request {
    pub fn write(tid: usize, data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Write { tid, data, locations }
    }

    pub fn read(context: Ctx, data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Read { context, data, locations }
    }

    pub fn serialize(tid: usize, data: Vec<u8>, hash: [u8; 20]) -> Request {
        Request::Serialize { tid, data, hash }
    }

    pub fn delete(tid: usize, hash: [u8; 20]) -> Request {
        Request::Delete { tid, hash }
    }

    pub fn shutdown() -> Request {
        Request::Shutdown
    }

    pub fn execute(self) -> io::Result<Option<Response>> {
        let sd = &CONFIG.session;
        match self {
            Request::Write { data, locations, .. } => {
                for loc in locations {
                    fs::OpenOptions::new().write(true).open(&loc.file).and_then(|mut f| {
                        f.seek(SeekFrom::Start(loc.offset)).map(|_| f)
                    }).and_then(|mut f| {
                        f.write(&data[loc.start..loc.end])
                    })?;
                }
            }
            Request::Read { context, mut data, locations, .. } =>  {
                for loc in locations {
                    fs::OpenOptions::new().read(true).open(&loc.file).and_then(|mut f| {
                        f.seek(SeekFrom::Start(loc.offset)).map(|_| f)
                    }).and_then(|mut f| {
                        f.read(&mut data[loc.start..loc.end])
                    })?;
                }
                let data = Arc::new(data);
                return Ok(Some(Response::read(context, data)))
            }
            Request::Serialize { data, hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(torrent_name(&hash));
                fs::OpenOptions::new().write(true).create(true).open(&pb).and_then(|mut f| {
                    f.write(&data)
                })?;
            }
            Request::Delete { hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(torrent_name(&hash));
                fs::remove_file(pb)?;
            }
            Request::Validate { info, .. } => {
            }
            Request::Shutdown => unreachable!(),
        }
        Ok(None)
    }

    pub fn tid(&self) -> usize {
        match *self {
            Request::Serialize { tid, .. }
            | Request::Validate { tid, .. }
            | Request::Delete { tid, .. }
            | Request::Write { tid, .. } => tid,
            Request::Read { ref context, .. } => context.tid,
            Request::Shutdown => unreachable!(),
        }
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

pub enum Response {
    Read { context: Ctx, data: Arc<Box<[u8; 16384]>> },
    Error { tid: usize, err: io::Error, }
}

impl Response {
    pub fn read(context: Ctx, data: Arc<Box<[u8; 16384]>>) -> Response {
        Response::Read { context, data }
    }

    pub fn error(tid: usize, err: io::Error) -> Response {
        Response::Error { tid, err }
    }

    pub fn tid(&self) -> usize {
        match *self {
            Response::Read { ref context, .. } => context.tid,
            Response::Error { tid, .. } => tid
        }
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "disk::Response")
    }
}

impl Disk {
    pub fn new(queue: mpsc::Receiver<Request>, l: Logger) -> Disk {
        let threads = 2;
        Disk {
            queue, l,
            pool: ThreadPool::new_with_name("disk_pool".into(), threads),
            threads,
        }
    }

    pub fn run(&mut self) {
        let sd = &CONFIG.session;
        fs::create_dir_all(sd).unwrap();
        debug!(self.l, "Initialized!");
        loop {
            match self.queue.recv() {
                Ok(Request::Shutdown) => {
                    break
                }
                Ok(r) => {
                    // Adjust the pool size
                    if self.pool.active_count() == self.threads {
                        self.threads += 1;
                        debug!(self.l, "Increasing disk pool to {:?}", self.threads);
                    } else if self.pool.active_count() + 2 < self.threads {
                        self.threads -= 1;
                        debug!(self.l, "Decreasing disk pool to {:?}", self.threads);
                    }
                    self.pool.set_num_threads(self.threads);
                    trace!(self.l, "Handling disk job!");
                    self.pool.execute(move || {
                        let tid = r.tid();
                        match r.execute() {
                            Ok(Some(r)) => {
                                CONTROL.disk_tx.lock().unwrap().send(r).unwrap();
                            }
                            Ok(None) => { }
                            Err(e) => {
                                CONTROL.disk_tx.lock().unwrap().send(Response::error(tid, e)).unwrap();
                            }
                        }
                    });
                }
                _ => break,
            }
        }
    }
}

pub fn start(l: Logger) -> Handle {
    debug!(l, "Initializing!");
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        Disk::new(rx, l.clone()).run();
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
        debug!(l, "Shutdown!");
    });
    Handle { tx }
}
