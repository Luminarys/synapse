mod job;
mod cache;

pub use self::job::Request;
pub use self::job::Response;
pub use self::job::Location;
pub use self::job::Ctx;

use std::collections::VecDeque;
use std::{fs, thread, io};

use amy;

use self::job::JobRes;
use self::cache::FileCache;
use {handle, CONFIG};
use util::UHashMap;

const POLL_INT_MS: usize = 1000;
const JOB_TIME_SLICE: u64 = 150;
const EXDEV: i32 = 18;
const MAX_CHAINED_OPS: usize = 128;

pub struct Disk {
    poll: amy::Poller,
    reg: amy::Registrar,
    ch: handle::Handle<Request, Response>,
    jobs: amy::Receiver<Request>,
    files: FileCache,
    active: VecDeque<Request>,
    blocked: UHashMap<Request>,
}


impl Disk {
    pub fn new(
        poll: amy::Poller,
        reg: amy::Registrar,
        ch: handle::Handle<Request, Response>,
        jobs: amy::Receiver<Request>,
    ) -> Disk {
        Disk {
            poll,
            reg,
            ch,
            jobs,
            files: FileCache::new(),
            active: VecDeque::new(),
            blocked: UHashMap::default(),
        }
    }

    pub fn run(&mut self) {
        let sd = &CONFIG.disk.session;
        fs::create_dir_all(sd).unwrap();

        loop {
            match self.poll.wait(POLL_INT_MS) {
                Ok(v) => {
                    if self.handle_events() {
                        break;
                    }
                    for ev in v {
                        if let Some(r) = self.blocked.remove(&ev.id) {
                            self.active.push_back(r);
                        }
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
        let mut rotate = 1;
        while let Some(j) = self.active.pop_front() {
            let tid = j.tid();
            match j.execute(&mut self.files) {
                Ok(JobRes::Resp(r)) => {
                    self.ch.send(r).ok();
                }
                Ok(JobRes::Paused(s)) => {
                    if rotate % 3 == 0 {
                        self.active.push_back(s);
                    } else {
                        self.active.push_front(s);
                    }
                }
                Ok(JobRes::Blocked((id, s))) => {
                    self.blocked.insert(id, s);
                }
                Ok(JobRes::Done) => {}
                Err(e) => {
                    if let Some(t) = tid {
                        self.ch.send(Response::error(t, e)).ok();
                    } else {
                        error!("Disk job failed: {}", e);
                    }
                }
            }
            match self.poll.wait(0) {
                Ok(v) => {
                    if self.handle_events() {
                        return true;
                    }
                    for ev in v {
                        if let Some(r) = self.blocked.remove(&ev.id) {
                            self.active.push_back(r);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to poll for events: {:?}", e);
                }
            }
            rotate += 1;
        }
        false
    }

    pub fn handle_events(&mut self) -> bool {
        loop {
            match self.ch.recv() {
                Ok(Request::Shutdown) => {
                    return true;
                }
                Ok(mut r) => {
                    trace!("Handling disk job!");
                    let tid = r.tid();
                    if let Err(e) = r.register(&self.reg) {
                        if let Some(t) = tid {
                            self.ch.send(Response::error(t, e)).ok();
                        }
                    }
                    match r.execute(&mut self.files) {
                        Ok(JobRes::Resp(r)) => {
                            self.ch.send(r).ok();
                        }
                        Ok(JobRes::Paused(s)) => {
                            self.active.push_back(s);
                        }
                        Ok(JobRes::Blocked((id, s))) => {
                            self.blocked.insert(id, s);
                        }
                        Ok(JobRes::Done) => {}
                        Err(e) => {
                            if let Some(t) = tid {
                                self.ch.send(Response::error(t, e)).ok();
                            }
                        }
                    }
                }
                _ => break,
            }
        }
        while let Ok(mut r) = self.jobs.try_recv() {
            if r.register(&self.reg).is_err() {
                continue;
            }
            match r.execute(&mut self.files) {
                Ok(JobRes::Paused(s)) => {
                    self.active.push_back(s);
                }
                Err(e) => {
                    error!("Disk job failed: {}", e);
                }
                _ => {}
            }
        }
        false
    }
}

pub fn start(
    creg: &mut amy::Registrar,
) -> io::Result<(handle::Handle<Response, Request>, amy::Sender<Request>, thread::JoinHandle<()>)> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar()?;
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    let (tx, rx) = reg.channel()?;
    let h = dh.run("disk", move |h| Disk::new(poll, reg, h, rx).run())?;
    Ok((ch, tx, h))
}
