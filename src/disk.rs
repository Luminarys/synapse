use std::sync::Arc;
use std::{fs, fmt, path, time};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;
use torrent::Info;
use slog::Logger;
use util::torrent_name;
use threadpool::ThreadPool;
use sha1::Sha1;
use amy;
use {handle, CONFIG};

const MAX_THREADS: usize = 10;
const BACKOFF_MS: u64 = 50;
const POLL_INT_MS: usize = 10;

pub struct Disk {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    l: Logger,
    pool: ThreadPool,
    threads: usize,
    backoff: time::Instant,
}

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

    pub fn validate(tid: usize, info: Arc<Info>) -> Request {
        Request::Validate { tid, info }
    }

    pub fn delete(tid: usize, hash: [u8; 20]) -> Request {
        Request::Delete { tid, hash }
    }

    pub fn shutdown() -> Request {
        Request::Shutdown
    }

    pub fn execute(self) -> io::Result<Option<Response>> {
        let sd = &CONFIG.session;
        let dd = &CONFIG.directory;
        match self {
            Request::Write { data, locations, .. } => {
                let mut pb = path::PathBuf::from(dd);
                for loc in locations {
                    pb.push(&loc.file);
                    let mut f = fs::OpenOptions::new().write(true).open(&pb)?;
                    f.seek(SeekFrom::Start(loc.offset))?;
                    f.write(&data[loc.start..loc.end])?;
                    pb.pop();
                }
            }
            Request::Read { context, mut data, locations, .. } =>  {
                let mut pb = path::PathBuf::from(dd);
                for loc in locations {
                    pb.push(&loc.file);
                    let mut f = fs::OpenOptions::new().read(true).open(&pb)?;
                    f.seek(SeekFrom::Start(loc.offset))?;
                    f.read(&mut data[loc.start..loc.end])?;
                    pb.pop();
                }
                let data = Arc::new(data);
                return Ok(Some(Response::read(context, data)))
            }
            Request::Serialize { data, hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(torrent_name(&hash));
                let mut f = fs::OpenOptions::new().write(true).create(true).open(&pb)?;
                f.write(&data)?;
            }
            Request::Delete { hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(torrent_name(&hash));
                fs::remove_file(pb)?;
            }
            Request::Validate { tid, info } => {
                let mut invalid = Vec::new();
                let mut buf = vec![0u8; info.piece_len as usize];
                let mut pb = path::PathBuf::from(dd);

                let mut init_locs = info.piece_disk_locs(0);
                let mut cf = init_locs.remove(0).file;
                pb.push(&cf);
                let mut f = fs::OpenOptions::new().read(true).open(&pb)?;

                for i in 0..info.pieces() {
                    let mut hasher = Sha1::new();
                    let locs = info.piece_disk_locs(i);
                    for loc in locs {
                        if &loc.file != &cf {
                            pb.pop();
                            pb.push(&loc.file);
                            f = fs::OpenOptions::new().read(true).open(&pb)?;
                            cf = loc.file;
                        }
                        let amnt = f.read(&mut buf)?;
                        hasher.update(&buf[0..amnt]);
                    }
                    let hash = hasher.digest().bytes();
                    if &hash[..] != &info.hashes[i as usize][..] {
                        invalid.push(i);
                    }
                }
                return Ok(Some(Response::validation_complete(tid, invalid)));
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

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "disk::Request")
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
    ValidationComplete { tid: usize, invalid: Vec<u32>, },
    Error { tid: usize, err: io::Error, }
}

impl Response {
    pub fn read(context: Ctx, data: Arc<Box<[u8; 16384]>>) -> Response {
        Response::Read { context, data }
    }

    pub fn error(tid: usize, err: io::Error) -> Response {
        Response::Error { tid, err }
    }

    pub fn validation_complete(tid: usize, invalid: Vec<u32>) -> Response {
        Response::ValidationComplete { tid, invalid }
    }

    pub fn tid(&self) -> usize {
        match *self {
            Response::Read { ref context, .. } => context.tid,
            Response::ValidationComplete { tid, .. }
            | Response::Error { tid, .. } => tid
        }
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "disk::Response")
    }
}

impl Disk {
    pub fn new(poll: amy::Poller, ch: handle::Handle<Request, Response>, l: Logger) -> Disk {
        let threads = 2;
        Disk {
            poll,
            ch,
            l,
            pool: ThreadPool::new_with_name("disk_pool".into(), threads),
            threads,
            backoff: time::Instant::now(),
        }
    }

    pub fn run(&mut self) {
        let sd = &CONFIG.session;
        fs::create_dir_all(sd).unwrap();

        loop {
            match self.poll.wait(POLL_INT_MS) {
                Ok(_) => {
                    if self.handle_events() {
                        break;
                    }
                }
                Err(e) => {
                    warn!(self.l, "Failed to poll for events: {:?}", e);
                }
            }
        }

        self.pool.join();
    }

    pub fn handle_events(&mut self) -> bool {
        loop {
            match self.ch.recv() {
                Ok(Request::Shutdown) => {
                    return true;
                }
                Ok(r) => {
                    // Adjust the pool size
                    if self.pool.active_count() == self.threads && self.pool.active_count() < MAX_THREADS {
                        debug!(self.l, "Increasing disk pool to {:?}", self.threads);
                        self.threads += 1;
                    } else if self.pool.active_count() + 2 < self.threads && self.backoff.elapsed() > time::Duration::from_millis(BACKOFF_MS) {
                        self.backoff = time::Instant::now();
                        debug!(self.l, "Decreasing disk pool to {:?}", self.threads);
                        self.threads -= 1;
                    }
                    self.pool.set_num_threads(self.threads);
                    trace!(self.l, "Handling disk job!");
                    let tx = self.ch.tx.try_clone().unwrap();
                    self.pool.execute(move || {
                        let tid = r.tid();
                        match r.execute() {
                            Ok(Some(r)) => {
                                tx.send(r).unwrap();
                            }
                            Ok(None) => { }
                            Err(e) => {
                                tx.send(Response::error(tid, e)).unwrap();
                            }
                        }
                    });
                }
                _ => { break },
            }
        }
        false
    }
}

pub fn start(creg: &mut amy::Registrar) -> io::Result<handle::Handle<Response, Request>> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar()?;
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    dh.run("disk", move |h, l| Disk::new(poll, h, l).run());
    Ok(ch)
}
