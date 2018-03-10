mod job;
mod cache;

pub use self::job::Request;
pub use self::job::Response;
pub use self::job::Location;
pub use self::job::Ctx;

use std::collections::VecDeque;
use std::{fs, io, thread};

use amy;

use self::job::JobRes;
use self::cache::FileCache;
use {handle, CONFIG};
use util::UHashMap;

const POLL_INT_MS: usize = 1000;
const JOB_TIME_SLICE: u64 = 150;

pub struct Disk {
    poll: amy::Poller,
    reg: amy::Registrar,
    ch: handle::Handle<Request, Response>,
    jobs: amy::Receiver<Request>,
    files: FileCache,
    active: VecDeque<Request>,
    sequential: VecDeque<Request>,
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
            sequential: VecDeque::new(),
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
                            self.enqueue_req(r);
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

    fn enqueue_req(&mut self, req: Request) {
        if req.concurrent() || self.active.iter().find(|r| !r.concurrent()).is_none() {
            self.active.push_back(req);
        } else {
            self.sequential.push_back(req);
        }
    }

    fn handle_active(&mut self) -> bool {
        let mut rotate = 1;
        while let Some(j) = self.active.pop_front() {
            let tid = j.tid();
            let seq = !j.concurrent();
            let mut done = false;
            match j.execute(&mut self.files) {
                Ok(JobRes::Resp(r)) => {
                    done = true;
                    self.ch.send(r).ok();
                }
                Ok(JobRes::Update(s, r)) => {
                    self.ch.send(r).ok();
                    if rotate % 3 == 0 {
                        self.active.push_back(s);
                    } else {
                        self.active.push_front(s);
                    }
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
                Ok(JobRes::Done) => {
                    done = true;
                }
                Err(e) => {
                    done = true;
                    if let Some(t) = tid {
                        self.ch.send(Response::error(t, e)).ok();
                    } else {
                        error!("Disk job failed: {}", e);
                    }
                }
            }
            if done && seq {
                self.sequential
                    .pop_front()
                    .map(|r| self.active.push_back(r));
            }
            match self.poll.wait(0) {
                Ok(v) => {
                    if self.handle_events() {
                        return true;
                    }
                    for ev in v {
                        if let Some(r) = self.blocked.remove(&ev.id) {
                            self.enqueue_req(r);
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
                    self.enqueue_req(r);
                }
                _ => break,
            }
        }
        while let Ok(mut r) = self.jobs.try_recv() {
            if r.register(&self.reg).is_err() {
                continue;
            }
            self.enqueue_req(r);
        }
        false
    }
}

pub fn start(
    creg: &mut amy::Registrar,
) -> io::Result<
    (
        handle::Handle<Response, Request>,
        amy::Sender<Request>,
        thread::JoinHandle<()>,
    ),
> {
    let poll = amy::Poller::new()?;
    let mut reg = poll.get_registrar();
    let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
    let (tx, rx) = reg.channel()?;
    let h = dh.run("disk", move |h| Disk::new(poll, reg, h, rx).run())?;
    Ok((ch, tx, h))
}
