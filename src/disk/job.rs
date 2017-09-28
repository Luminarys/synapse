use std::sync::Arc;
use std::{fs, fmt, path, time, mem};
use std::io::{self, Seek, SeekFrom, Write, Read};
use std::path::{Path, PathBuf};
use std::net::TcpStream;

use fs_extra;
use sha1;
use amy;
use byteorder::{BigEndian, ReadBytesExt};

use super::{EXDEV, JOB_TIME_SLICE, FileCache};
use torrent::{Info, LocIter, PeerConn, Block};
use util::{hash_to_id, io_err, awrite, IOR, FHashSet};
use CONFIG;

pub struct Location {
    pub file: usize,
    pub offset: u64,
    pub start: usize,
    pub end: usize,
    info: Arc<Info>,
}

pub enum Request {
    Create {
        tid: usize,
        priorities: Vec<u8>,
        path: Option<String>,
        info: Arc<Info>,
    },
    Write {
        tid: usize,
        data: Box<[u8; 16_384]>,
        locations: LocIter,
        path: Option<String>,
    },
    Read {
        data: Box<[u8; 16_384]>,
        locations: LocIter,
        context: Ctx,
        path: Option<String>,
    },
    BatchWrite {
        tid: usize,
        pid: usize,
        info: Arc<Info>,
        path: Option<String>,
        conn: PeerConn,
        id: usize,
        blocks: FHashSet<Block>,
        state: WriteState,
    },
    BatchRead {
        tid: usize,
        pid: usize,
        info: Arc<Info>,
        path: Option<String>,
        conn: PeerConn,
        id: usize,
        blocks: FHashSet<Block>,
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
    Download {
        client: TcpStream,
        path: String,
        offset: Option<u64>,
        id: usize,
    },
    Shutdown,
}

pub enum WriteState {
    Metadata([u8; 13]),
    Data { locs: LocIter, cloc: Location },
}

pub enum Response {
    Read {
        context: Ctx,
        data: Arc<Box<[u8; 16_384]>>,
    },
    BatchWrite {
        tid: usize,
        pid: usize,
        conn: PeerConn,
        blocks: FHashSet<Block>,
    },
    ValidationComplete { tid: usize, invalid: Vec<u32> },
    Moved { tid: usize, path: String },
    Error {
        tid: usize,
        pid: Option<usize>,
        err: io::Error,
    },
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
    pub fn create(
        tid: usize,
        info: Arc<Info>,
        path: Option<String>,
        priorities: Vec<u8>,
    ) -> Request {
        Request::Create {
            tid,
            info,
            path,
            priorities,
        }
    }

    pub fn write(
        tid: usize,
        data: Box<[u8; 16_384]>,
        locations: LocIter,
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
        locations: LocIter,
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

    pub fn download(client: TcpStream, path: String) -> Request {
        Request::Download {
            client,
            path,
            offset: None,
            id: 0,
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
            Request::Create {
                info,
                path,
                priorities,
                ..
            } => {
                if let Some(p) = path {
                    info.create_files(&path::Path::new(&p), &priorities)?;
                } else {
                    info.create_files(&path::Path::new(&dd), &priorities)?;
                }
            }
            Request::BatchWrite {
                info,
                path,
                mut blocks,
                mut conn,
                mut state,
                id,
                tid,
                pid,
            } => {
                let start = time::Instant::now();
                while start.elapsed() < time::Duration::from_millis(JOB_TIME_SLICE) {
                    match state {
                        WriteState::Metadata(mut buf) => {
                            match conn.sock().peek(&mut buf) {
                                Ok(0) => return io_err("EOF"),
                                Ok(a) if a == buf.len() => {
                                    let len = (&buf[0..4]).read_u32::<BigEndian>().unwrap();
                                    if buf[4] != 7 {
                                        return Ok(JobRes::Resp(Response::BatchWrite {
                                            tid,
                                            pid,
                                            conn,
                                            blocks,
                                        }));
                                    }
                                    let index = (&buf[5..9]).read_u32::<BigEndian>().unwrap();
                                    let offset = (&buf[9..13]).read_u32::<BigEndian>().unwrap();
                                    if blocks.remove(&Block { index, offset }) {
                                        let mut locs = Info::block_disk_locs(&info, index, offset);
                                        let cloc = locs.next().unwrap();
                                        state = WriteState::Data { locs, cloc };
                                    } else {
                                        return io_err("Bad block!");
                                    }
                                }
                                Ok(a) => {
                                    state = WriteState::Metadata(buf);
                                }
                                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                                    return Ok(JobRes::Blocked((
                                        id,
                                        Request::BatchWrite {
                                            pid,
                                            info,
                                            path,
                                            blocks,
                                            conn,
                                            id,
                                            tid,
                                            state: WriteState::Metadata(buf),
                                        },
                                    )));
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        WriteState::Data { mut locs, mut cloc } => {
                            let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                            pb.push(cloc.path());
                            let amnt = fc.get_file_range(
                                &pb,
                                cloc.offset,
                                (cloc.end - cloc.start),
                                false,
                                |b| match conn.sock_mut().read(b) {
                                    Ok(0) => io_err("EOF"),
                                    Ok(a) => Ok(Some(a)),
                                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => Ok(None),
                                    Err(e) => Err(e),
                                },
                            )??;
                            if let Some(a) = amnt {
                                cloc.start += a;
                                cloc.offset += a as u64;
                            } else {
                                return Ok(JobRes::Blocked((
                                    id,
                                    Request::BatchWrite {
                                        pid,
                                        info,
                                        path,
                                        blocks,
                                        conn,
                                        id,
                                        tid,
                                        state: WriteState::Data { locs, cloc },
                                    },
                                )));
                            }
                            if cloc.start == cloc.end {
                                match locs.next() {
                                    Some(loc) => state = WriteState::Data { cloc: loc, locs },
                                    None => {
                                        if blocks.is_empty() {
                                            return Ok(JobRes::Resp(Response::BatchWrite {
                                                conn,
                                                blocks,
                                                tid,
                                                pid,
                                            }));
                                        } else {
                                            state = WriteState::Metadata([0u8; 13])
                                        }
                                    }
                                }
                            } else {
                                state = WriteState::Data { cloc, locs }
                            }
                        }
                    }
                }
                return Ok(JobRes::Paused(Request::BatchWrite {
                    info,
                    path,
                    blocks,
                    conn,
                    state,
                    id,
                    tid,
                    pid,
                }));
            }
            Request::BatchRead { .. } => {
                unimplemented!();
            }
            Request::Write {
                data,
                locations,
                path,
                ..
            } => {
                for loc in locations {
                    let mut pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                    pb.push(loc.path());
                    fc.get_file_range(
                        &pb,
                        loc.offset,
                        (loc.end - loc.start),
                        false,
                        |b| { b.copy_from_slice(&data[loc.start..loc.end]); },
                    )?;
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
                    pb.push(loc.path());
                    fc.get_file_range(
                        &pb,
                        loc.offset,
                        (loc.end - loc.start),
                        true,
                        |b| {
                            (&mut data[loc.start..loc.end]).copy_from_slice(b);
                        },
                    )?;
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
                    start.elapsed() < time::Duration::from_millis(JOB_TIME_SLICE)
                {
                    let mut valid = true;
                    let mut ctx = sha1::Sha1::new();
                    let locs = Info::piece_disk_locs(&info, idx);
                    let mut pos = 0;
                    for loc in locs {
                        if loc.path() != cf {
                            pb = path::PathBuf::from(path.as_ref().unwrap_or(dd));
                            pb.push(loc.path());
                            f = fs::OpenOptions::new().read(true).open(&pb);
                            cf = loc.path().to_owned();
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
            Request::Download {
                mut client,
                path,
                offset,
                id,
            } => {
                if let Some(o) = offset {
                    let mut no = o;
                    let start = time::Instant::now();
                    let mut buf: [u8; 16_384] = unsafe { mem::uninitialized() };
                    'read: while start.elapsed() < time::Duration::from_millis(JOB_TIME_SLICE) {
                        let r = fc.get_file(path::Path::new(&path), |f| {
                            f.seek(SeekFrom::Start(no))?;
                            loop {
                                match f.read(&mut buf) {
                                    Ok(r) => return Ok(r),
                                    Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
                                        continue
                                    }
                                    Err(e) => return Err(e),
                                }
                            }
                        })?;
                        if r == 0 {
                            return Ok(JobRes::Done);
                        }
                        'write: loop {
                            // Need the mod here because after the first 16 KiBs complete
                            // no will be too big
                            let b = &mut buf[(no - o) as usize % 16_384..r];

                            match awrite(b, &mut client) {
                                IOR::Complete => {
                                    no += b.len() as u64;
                                    continue 'read;
                                }
                                IOR::Incomplete(w) => no += w as u64,
                                IOR::Blocked => {
                                    return Ok(JobRes::Blocked((
                                        id,
                                        Request::Download {
                                            client,
                                            path,
                                            offset: Some(no),
                                            id,
                                        },
                                    )))
                                }
                                IOR::EOF => return io_err("EOF"),
                                IOR::Err(e) => return Err(e),
                            }
                        }
                    }
                    return Ok(JobRes::Paused(Request::Download {
                        client,
                        path,
                        offset: Some(no),
                        id,
                    }));
                } else {
                    fc.get_file(path::Path::new(&path), |f| {
                        let len = f.metadata()?.len();
                        let lines = vec![
                            format!("HTTP/1.1 200 OK"),
                            format!("Content-Length: {}", len),
                            format!("Content-Type: {}", "application/octet-stream"),
                            format!(
                                "Content-Disposition: attachment; filename=\"{}\"",
                                path::Path::new(&path)
                                    .file_name()
                                    .unwrap()
                                    .to_string_lossy()
                            ),
                            format!("Connection: {}", "Close"),
                            format!("\r\n"),
                        ];
                        let data = lines.join("\r\n");
                        client.write_all(data.as_bytes())?;
                        Ok(())
                    })?;
                    return Ok(JobRes::Paused(Request::Download {
                        client,
                        path,
                        offset: Some(0),
                        id,
                    }));
                }
            }
            Request::Shutdown => unreachable!(),
        }
        Ok(JobRes::Done)
    }

    pub fn register(&mut self, reg: &amy::Registrar) -> io::Result<()> {
        match *self {
            Request::Download {
                ref client,
                ref mut id,
                ..
            } => {
                *id = reg.register(client, amy::Event::Write)?;
                Ok(())
            }
            Request::BatchWrite {
                ref conn,
                ref mut id,
                ..
            } => {
                *id = reg.register(conn.sock(), amy::Event::Read)?;
                Ok(())
            }
            Request::BatchRead {
                ref conn,
                ref mut id,
                ..
            } => {
                *id = reg.register(conn.sock(), amy::Event::Write)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    pub fn tid(&self) -> Option<usize> {
        match *self {
            Request::Serialize { tid, .. } |
            Request::Validate { tid, .. } |
            Request::Delete { tid, .. } |
            Request::Move { tid, .. } |
            Request::Create { tid, .. } |
            Request::BatchWrite { tid, .. } |
            Request::BatchRead { tid, .. } |
            Request::Write { tid, .. } => Some(tid),
            Request::Read { ref context, .. } => Some(context.tid),
            Request::WriteFile { .. } |
            Request::Download { .. } |
            Request::Shutdown => None,
        }
    }

    pub fn pid(&self) -> Option<usize> {
        match *self {
            Request::BatchWrite { pid, .. } |
            Request::BatchRead { pid, .. } => Some(pid),
            _ => None,
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "disk::Request")
    }
}

impl Location {
    pub fn new(file: usize, offset: u64, start: u64, end: u64, info: Arc<Info>) -> Location {
        Location {
            file,
            offset,
            start: start as usize,
            end: end as usize,
            info,
        }
    }

    pub fn path(&self) -> &Path {
        &self.info.files[self.file].path
    }
}

impl fmt::Debug for Location {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "disk::Location {{ file: {}, off: {}, s: {}, e: {} }}",
            self.file,
            self.offset,
            self.start,
            self.end
        )
    }
}

impl Response {
    pub fn read(context: Ctx, data: Arc<Box<[u8; 16_384]>>) -> Response {
        Response::Read { context, data }
    }

    pub fn error(tid: usize, pid: Option<usize>, err: io::Error) -> Response {
        Response::Error { tid, err, pid }
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
            Response::BatchWrite { tid, .. } |
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
