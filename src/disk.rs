use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use std::{fs, fmt, path};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;
use torrent::Info;
use slog::Logger;
use util::hash_to_id;
use ring::digest;
use amy;
use {handle, CONFIG};

const POLL_INT_MS: usize = 1000;

pub struct Disk {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    l: Logger,
    files: FileCache,
}

#[derive(Clone)]
struct FileCache {
    files: Arc<Mutex<HashMap<path::PathBuf, fs::File>>>,
}

pub enum Request {
    Write {
        tid: usize,
        data: Box<[u8; 16384]>,
        locations: Vec<Location>,
    },
    Read {
        data: Box<[u8; 16384]>,
        locations: Vec<Location>,
        context: Ctx,
    },
    Serialize {
        tid: usize,
        data: Vec<u8>,
        hash: [u8; 20],
    },
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
        Ctx {
            pid,
            tid,
            idx,
            begin,
            length,
        }
    }
}

impl FileCache {
    pub fn new() -> FileCache {
        FileCache { files: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn get_file<F: FnOnce(&mut fs::File) -> io::Result<()>>(
        &self,
        path: &path::Path,
        f: F,
    ) -> io::Result<()> {
        let mut d = self.files.lock().unwrap();
        if d.contains_key(path) {
            f(d.get_mut(path).unwrap())?;
        } else {
            // TODO: LRU maybe?
            if d.len() >= CONFIG.net.max_open_files {
                let removal = d.iter().map(|(id, _)| id.clone()).next().unwrap();
                d.remove(&removal);
            }
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(path)?;
            f(&mut file)?;
            d.insert(path.to_path_buf(), file);
        }
        Ok(())
    }
}

impl Request {
    pub fn write(tid: usize, data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Write {
            tid,
            data,
            locations,
        }
    }

    pub fn read(context: Ctx, data: Box<[u8; 16384]>, locations: Vec<Location>) -> Request {
        Request::Read {
            context,
            data,
            locations,
        }
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

    fn execute(self, fc: FileCache) -> io::Result<Option<Response>> {
        let sd = &CONFIG.disk.session;
        let dd = &CONFIG.disk.directory;
        match self {
            Request::Write { data, locations, .. } => {
                let mut pb = path::PathBuf::from(dd);
                for loc in locations {
                    pb.push(&loc.file);
                    fc.get_file(&pb, |f| {
                        f.seek(SeekFrom::Start(loc.offset))?;
                        f.write(&data[loc.start..loc.end])?;
                        Ok(())
                    })?;
                    pb.pop();
                }
            }
            Request::Read {
                context,
                mut data,
                locations,
                ..
            } => {
                let mut pb = path::PathBuf::from(dd);
                for loc in locations {
                    pb.push(&loc.file);
                    fc.get_file(&pb, |f| {
                        f.seek(SeekFrom::Start(loc.offset))?;
                        f.read(&mut data[loc.start..loc.end])?;
                        Ok(())
                    })?;
                    pb.pop();
                }
                let data = Arc::new(data);
                return Ok(Some(Response::read(context, data)));
            }
            Request::Serialize { data, hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(hash_to_id(&hash));
                let mut f = fs::OpenOptions::new().write(true).create(true).open(&pb)?;
                f.write(&data)?;
            }
            Request::Delete { hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(hash_to_id(&hash));
                fs::remove_file(pb)?;
            }
            Request::Validate { tid, info } => {
                let mut invalid = Vec::new();
                let mut buf = vec![0u8; info.piece_len as usize];
                let mut pb = path::PathBuf::from(dd);

                let mut init_locs = info.piece_disk_locs(0);
                let mut cf = init_locs.remove(0).file;
                pb.push(&cf);
                let mut f = fs::OpenOptions::new().read(true).open(&pb);

                for i in 0..info.pieces() {
                    let mut valid = true;
                    let mut ctx = digest::Context::new(&digest::SHA1);
                    let locs = info.piece_disk_locs(i);
                    for loc in locs {
                        if &loc.file != &cf {
                            pb.pop();
                            pb.push(&loc.file);
                            f = fs::OpenOptions::new().read(true).open(&pb);
                            cf = loc.file;
                        }
                        if let Ok(Ok(amnt)) = f.as_mut().map(|file| file.read(&mut buf)) {
                            ctx.update(&buf[0..amnt]);
                        } else {
                            valid = false;
                        }
                    }
                    let digest = ctx.finish();
                    if !valid || digest.as_ref() != &info.hashes[i as usize][..] {
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
            Request::Serialize { tid, .. } |
            Request::Validate { tid, .. } |
            Request::Delete { tid, .. } |
            Request::Write { tid, .. } => tid,
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
        Location {
            file,
            offset,
            start,
            end,
        }
    }
}

pub enum Response {
    Read {
        context: Ctx,
        data: Arc<Box<[u8; 16384]>>,
    },
    ValidationComplete { tid: usize, invalid: Vec<u32> },
    Error { tid: usize, err: io::Error },
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
            Response::ValidationComplete { tid, .. } |
            Response::Error { tid, .. } => tid,
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
        Disk {
            poll,
            ch,
            l,
            files: FileCache::new(),
        }
    }

    pub fn run(&mut self) {
        let sd = &CONFIG.disk.session;
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

    }

    pub fn handle_events(&mut self) -> bool {
        loop {
            match self.ch.recv() {
                Ok(Request::Shutdown) => {
                    return true;
                }
                Ok(r) => {
                    trace!(self.l, "Handling disk job!");
                    let tid = r.tid();
                    match r.execute(self.files.clone()) {
                        Ok(Some(r)) => {
                            self.ch.send(r).unwrap();
                        }
                        Ok(None) => {}
                        Err(e) => {
                            self.ch.send(Response::error(tid, e)).unwrap();
                        }
                    }
                }
                _ => break,
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
