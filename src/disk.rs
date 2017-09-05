use std::sync::Arc;
use std::collections::{HashMap, VecDeque};
use std::{fs, fmt, path, time, thread};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;
use torrent::Info;
use util::hash_to_id;
use ring::digest;
use amy;
use {handle, CONFIG};

const POLL_INT_MS: usize = 1000;
const JOB_TIME_SLICE: u64 = 1;

pub struct Disk {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    files: FileCache,
    active: VecDeque<Request>,
}

struct FileCache {
    files: HashMap<path::PathBuf, fs::File>,
}

pub enum Request {
    Write {
        tid: usize,
        data: Box<[u8; 16_384]>,
        locations: Vec<Location>,
        path: Option<String>,
    },
    Read {
        data: Box<[u8; 16_384]>,
        locations: Vec<Location>,
        context: Ctx,
        path: Option<String>,
    },
    Serialize {
        tid: usize,
        data: Vec<u8>,
        hash: [u8; 20],
    },
    Delete {
        tid: usize,
        hash: [u8; 20],
        files: Vec<PathBuf>,
        path: Option<String>,
    },

    Validate {
        tid: usize,
        info: Arc<Info>,
        path: Option<String>,
        idx: u32,
        invalid: Vec<u32>,
    },
    Shutdown,
}

enum JobRes {
    Resp(Response),
    Done,
    Paused(Request),
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
        FileCache { files: HashMap::new() }
    }

    pub fn get_file<F: FnMut(&mut fs::File) -> io::Result<()>>(
        &mut self,
        path: &path::Path,
        mut f: F,
    ) -> io::Result<()> {
        let hit = if let Some(file) = self.files.get_mut(path) {
            f(file)?;
            true
        } else {
            false
        };
        if !hit {
            // TODO: LRU maybe?
            if self.files.len() >= CONFIG.net.max_open_files {
                let removal = self.files.iter().map(|(id, _)| id.clone()).next().unwrap();
                self.files.remove(&removal);
            }
            fs::create_dir_all(path.parent().unwrap())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(path)?;
            f(&mut file)?;
            self.files.insert(path.to_path_buf(), file);
        }
        Ok(())
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        self.files.remove(path);
    }
}

impl Request {
    pub fn write(
        tid: usize,
        data: Box<[u8; 16_384]>,
        locations: Vec<Location>,
        path: Option<String>,
    ) -> Request {
        Request::Write {
            tid,
            data,
            locations,
            path,
        }
    }

    pub fn read(
        context: Ctx,
        data: Box<[u8; 16_384]>,
        locations: Vec<Location>,
        path: Option<String>,
    ) -> Request {
        Request::Read {
            context,
            data,
            locations,
            path,
        }
    }

    pub fn serialize(tid: usize, data: Vec<u8>, hash: [u8; 20]) -> Request {
        Request::Serialize { tid, data, hash }
    }

    pub fn validate(tid: usize, info: Arc<Info>, path: Option<String>) -> Request {
        Request::Validate {
            tid,
            info,
            path,
            idx: 0,
            invalid: Vec::new(),
        }
    }

    pub fn delete(
        tid: usize,
        hash: [u8; 20],
        files: Vec<PathBuf>,
        path: Option<String>,
    ) -> Request {
        Request::Delete {
            tid,
            hash,
            files,
            path,
        }
    }

    pub fn shutdown() -> Request {
        Request::Shutdown
    }

    fn execute(self, fc: &mut FileCache) -> io::Result<JobRes> {
        let sd = &CONFIG.disk.session;
        let dd = &CONFIG.disk.directory;
        match self {
            Request::Write {
                data,
                locations,
                path,
                ..
            } => {
                for loc in locations {
                    let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                    pb.push(&loc.file);
                    fc.get_file(&pb, |f| {
                        f.seek(SeekFrom::Start(loc.offset))?;
                        f.write_all(&data[loc.start..loc.end])?;
                        Ok(())
                    })?;
                }
            }
            Request::Read {
                context,
                mut data,
                locations,
                path,
                ..
            } => {
                for loc in locations {
                    let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                    pb.push(&loc.file);
                    fc.get_file(&pb, |f| {
                        f.seek(SeekFrom::Start(loc.offset))?;
                        f.read_exact(&mut data[loc.start..loc.end])?;
                        Ok(())
                    })?;
                }
                let data = Arc::new(data);
                return Ok(JobRes::Resp(Response::read(context, data)));
            }
            Request::Serialize { data, hash, .. } => {
                let mut pb = path::PathBuf::from(sd);
                pb.push(hash_to_id(&hash));
                let mut f = fs::OpenOptions::new().write(true).create(true).open(&pb)?;
                f.write_all(&data)?;
            }
            Request::Delete { hash, files, path, .. } => {
                let mut spb = path::PathBuf::from(sd);
                spb.push(hash_to_id(&hash));
                fs::remove_file(spb)?;

                for file in files {
                    let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                    pb.push(&file);
                    fc.remove_file(&pb);
                }
            }
            Request::Validate {
                tid,
                info,
                path,
                mut idx,
                mut invalid,
            } => {
                let mut buf = vec![0u8; info.piece_len as usize];
                let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                let mut cf = pb.clone();

                let mut f = fs::OpenOptions::new().read(true).open(&pb);

                let start = time::Instant::now();

                while idx < info.pieces() &&
                    start.elapsed() < time::Duration::from_secs(JOB_TIME_SLICE)
                {
                    let mut valid = true;
                    let mut ctx = digest::Context::new(&digest::SHA1);
                    let locs = info.piece_disk_locs(idx);
                    let mut pos = 0;
                    for loc in locs {
                        if loc.file != cf {
                            pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                            pb.push(&loc.file);
                            f = fs::OpenOptions::new().read(true).open(&pb);
                            cf = loc.file.clone();
                        }
                        // Because this is pausable/resumable, we need to seek to the proper
                        // file position.
                        f.as_mut()
                            .map(|file| file.seek(SeekFrom::Start(loc.offset)))
                            .ok();
                        if let Ok(Ok(amnt)) = f.as_mut().map(|file| file.read(&mut buf[pos..])) {
                            ctx.update(&buf[pos..pos + amnt]);
                            pos += amnt;
                        } else {
                            valid = false;
                        }
                    }
                    let digest = ctx.finish();
                    if !valid || digest.as_ref() != &info.hashes[idx as usize][..] {
                        invalid.push(idx);
                    }

                    idx += 1;
                }
                if idx == info.pieces() {
                    return Ok(JobRes::Resp(Response::validation_complete(tid, invalid)));
                } else {
                    return Ok(JobRes::Paused(Request::Validate {
                        tid,
                        info,
                        path,
                        idx,
                        invalid,
                    }));
                }
            }
            Request::Shutdown => unreachable!(),
        }
        Ok(JobRes::Done)
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
    pub fn new(file: PathBuf, offset: u64, start: u64, end: u64) -> Location {
        Location {
            file,
            offset,
            start: start as usize,
            end: end as usize,
        }
    }
}

pub enum Response {
    Read {
        context: Ctx,
        data: Arc<Box<[u8; 16_384]>>,
    },
    ValidationComplete { tid: usize, invalid: Vec<u32> },
    Error { tid: usize, err: io::Error },
}

impl Response {
    pub fn read(context: Ctx, data: Arc<Box<[u8; 16_384]>>) -> Response {
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
    pub fn new(poll: amy::Poller, ch: handle::Handle<Request, Response>) -> Disk {
        Disk {
            poll,
            ch,
            files: FileCache::new(),
            active: VecDeque::new(),
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
                    error!("Failed to poll for events: {:?}", e);
                }
            }
            if !self.active.is_empty() && self.handle_active() {
                break;
            }
        }
    }

    fn handle_active(&mut self) -> bool {
        while let Some(j) = self.active.pop_front() {
            let tid = j.tid();
            match j.execute(&mut self.files) {
                Ok(JobRes::Resp(r)) => {
                    self.ch.send(r).ok();
                }
                Ok(JobRes::Paused(s)) => {
                    self.active.push_front(s);
                }
                Ok(JobRes::Done) => {}
                Err(e) => {
                    self.ch.send(Response::error(tid, e)).ok();
                }
            }
            match self.poll.wait(0) {
                Ok(_) => {
                    if self.handle_events() {
                        return true;
                    }
                }
                Err(e) => {
                    error!("Failed to poll for events: {:?}", e);
                }
            }
        }
        false
    }

    pub fn handle_events(&mut self) -> bool {
        loop {
            match self.ch.recv() {
                Ok(Request::Shutdown) => {
                    return true;
                }
                Ok(r) => {
                    trace!("Handling disk job!");
                    let tid = r.tid();
                    match r.execute(&mut self.files) {
                        Ok(JobRes::Resp(r)) => {
                            self.ch.send(r).ok();
                        }
                        Ok(JobRes::Paused(s)) => {
                            self.active.push_back(s);
                        }
                        Ok(JobRes::Done) => {}
                        Err(e) => {
                            self.ch.send(Response::error(tid, e)).ok();
                        }
                    }
                }
                _ => break,
            }
        }
        false
    }
}

pub fn start(
    creg: &mut amy::Registrar,
) -> io::Result<(handle::Handle<Response, Request>, thread::JoinHandle<()>)> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar()?;
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    let h = dh.run("disk", move |h| Disk::new(poll, h).run())?;
    Ok((ch, h))
}
