use std::sync::Arc;
use std::{fs, fmt, path, time};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::PathBuf;

use fs_extra;
use sha1;

use super::{EXDEV, JOB_TIME_SLICE, FileCache};
use torrent::Info;
use util::{hash_to_id, io_err};
use CONFIG;

pub struct Location {
    pub file: PathBuf,
    pub offset: u64,
    pub start: usize,
    pub end: usize,
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
    WriteFile { data: Vec<u8>, path: PathBuf },
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

pub enum JobRes {
    Resp(Response),
    Done,
    Paused(Request),
    Blocked((usize, Request)),
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

    pub fn execute(self, fc: &mut FileCache) -> io::Result<JobRes> {
        let sd = &CONFIG.disk.session;
        let dd = &CONFIG.disk.directory;
        match self {
            Request::WriteFile { path, data } => {
                let mut p = path.clone();
                p.set_extension("temp");
                let res = fs::OpenOptions::new()
                    .write(true)
                    .create(true)
                    .open(&p)
                    .map(|mut f| f.write(&data[..]));
                match res {
                    Ok(Ok(_)) => {
                        fs::rename(&p, &path).ok();
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

    pub fn tid(&self) -> Option<usize> {
        match *self {
            Request::Serialize { tid, .. } |
            Request::Validate { tid, .. } |
            Request::Delete { tid, .. } |
            Request::Move { tid, .. } |
            Request::Write { tid, .. } => Some(tid),
            Request::Read { ref context, .. } => Some(context.tid),
            Request::WriteFile { .. } |
            Request::Shutdown => None,
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
