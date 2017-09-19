use std::sync::Arc;
use std::collections::{HashMap, VecDeque};
use std::{fs, fmt, path, time, thread};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;

use fs_extra;
use amy;
use sha1;

use torrent::Info;
use util::{hash_to_id, io_err};
use {handle, CONFIG};

const POLL_INT_MS: usize = 1000;
const JOB_TIME_SLICE: u64 = 1;
const EXDEV: i32 = 18;

pub struct Disk {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    jobs: amy::Receiver<Job>,
    files: FileCache,
    active: VecDeque<Request>,
}

pub struct Location {
    pub file: PathBuf,
    pub offset: u64,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct Job {
    pub data: Vec<u8>,
    pub path: PathBuf,
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
        artifacts: bool,
    },
    Move {
        tid: usize,
        from: String,
        to: String,
        target: String,
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

pub enum Response {
    Read {
        context: Ctx,
        data: Arc<Box<[u8; 16_384]>>,
    },
    ValidationComplete { tid: usize, invalid: Vec<u32> },
    Moved { tid: usize, path: String },
    Error { tid: usize, err: io::Error },
}

pub struct Ctx {
    pub pid: usize,
    pub tid: usize,
    pub idx: u32,
    pub begin: u32,
    pub length: u32,
}

enum JobRes {
    Resp(Response),
    Done,
    Paused(Request),
}

struct FileCache {
    files: HashMap<path::PathBuf, fs::File>,
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
        artifacts: bool,
    ) -> Request {
        Request::Delete {
            tid,
            hash,
            files,
            path,
            artifacts,
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
            Request::Move {
                tid,
                from,
                to,
                target,
            } => {
                let mut fp = PathBuf::from(&from);
                let mut tp = PathBuf::from(&to);
                fp.push(target.clone());
                tp.push(target);
                match fs::rename(&fp, &tp) {
                    Ok(_) => {}
                    // Cross filesystem move, try to copy then delete
                    Err(ref e) if e.raw_os_error() == Some(EXDEV) => {
                        match fs_extra::dir::copy(&fp, &tp, &fs_extra::dir::CopyOptions::new()) {
                            Ok(_) => {
                                fs::remove_dir_all(&fp)?;
                            }
                            Err(e) => {
                                fs::remove_dir_all(&tp)?;
                                error!("FS copy failed: {:?}", e);
                                return io_err("Failed to copy directory across filesystems!");
                            }
                        }
                    }
                    Err(e) => {
                        error!("FS rename failed: {:?}", e);
                        return Err(e);
                    }
                }
                return Ok(JobRes::Resp(Response::moved(tid, to)));
            }
            Request::Serialize { data, hash, .. } => {
                let mut temp = path::PathBuf::from(sd);
                temp.push(hash_to_id(&hash) + ".temp");
                let mut f = fs::OpenOptions::new().write(true).create(true).open(&temp)?;
                f.write_all(&data)?;
                let mut actual = path::PathBuf::from(sd);
                actual.push(hash_to_id(&hash));
                fs::rename(temp, actual)?;
            }
            Request::Delete {
                hash,
                files,
                path,
                artifacts,
                tid: _,
            } => {
                let mut spb = path::PathBuf::from(sd);
                spb.push(hash_to_id(&hash));
                fs::remove_file(spb)?;

                for file in files {
                    let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                    pb.push(&file);
                    fc.remove_file(&pb);
                    if artifacts {
                        if let Err(e) = fs::remove_file(&pb) {
                            error!("Failed to delete file: {:?}, {}", pb, e);
                        }
                    }
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
                    let mut ctx = sha1::Sha1::new();
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
            Request::Move { tid, .. } |
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

impl Response {
    pub fn read(context: Ctx, data: Arc<Box<[u8; 16_384]>>) -> Response {
        Response::Read { context, data }
    }

    pub fn error(tid: usize, err: io::Error) -> Response {
        Response::Error { tid, err }
    }

    pub fn moved(tid: usize, path: String) -> Response {
        Response::Moved { tid, path }
    }

    pub fn validation_complete(tid: usize, invalid: Vec<u32>) -> Response {
        Response::ValidationComplete { tid, invalid }
    }

    pub fn tid(&self) -> usize {
        match *self {
            Response::Read { ref context, .. } => context.tid,
            Response::ValidationComplete { tid, .. } |
            Response::Moved { tid, .. } |
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
    pub fn new(
        poll: amy::Poller,
        ch: handle::Handle<Request, Response>,
        jobs: amy::Receiver<Job>,
    ) -> Disk {
        Disk {
            poll,
            ch,
            jobs,
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
        while let Ok(j) = self.jobs.try_recv() {
            let mut p = j.path.clone();
            p.set_extension("temp");
            let res = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .open(&p)
                .map(|mut f| f.write(&j.data[..]));
            match res {
                Ok(Ok(_)) => {
                    fs::rename(&p, &j.path).ok();
                }
                Ok(Err(e)) => {
                    error!("Failed to write disk job: {}", e);
                    fs::remove_file(&p).ok();
                }
                Err(e) => {
                    error!("Failed to write disk job: {}", e);
                }
            }
        }
        false
    }
}

pub fn start(
    creg: &mut amy::Registrar,
) -> io::Result<(handle::Handle<Response, Request>, amy::Sender<Job>, thread::JoinHandle<()>)> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar()?;
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    let (tx, rx) = reg.channel()?;
    let h = dh.run("disk", move |h| Disk::new(poll, h, rx).run())?;
    Ok((ch, tx, h))
}
