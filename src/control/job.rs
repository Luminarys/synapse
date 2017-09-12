use torrent::{Torrent, Status};
use std::collections::HashMap;
use std::time;
use control::cio;

pub trait Job<T: cio::CIO> {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>);
}

pub struct JobManager<T: cio::CIO> {
    jobs: Vec<JobData<T>>,
}

struct JobData<T: cio::CIO> {
    job: Box<Job<T>>,
    last_updated: time::Instant,
    interval: time::Duration,
}

impl<T: cio::CIO> JobManager<T> {
    pub fn new() -> JobManager<T> {
        JobManager { jobs: Vec::new() }
    }

    pub fn add_job<J: Job<T> + 'static>(&mut self, job: J, interval: time::Duration) {
        self.jobs.push(JobData {
            job: Box::new(job),
            interval,
            last_updated: time::Instant::now(),
        })
    }

    pub fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for j in &mut self.jobs {
            if j.last_updated.elapsed() > j.interval {
                j.job.update(torrents);
                j.last_updated = time::Instant::now();
            }
        }
    }
}

pub struct TrackerUpdate;

impl<T: cio::CIO> Job<T> for TrackerUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.try_update_tracker();
        }
    }
}

pub struct UnchokeUpdate;

impl<T: cio::CIO> Job<T> for UnchokeUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_unchoked();
        }
    }
}

pub struct SessionUpdate;

impl<T: cio::CIO> Job<T> for SessionUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            if torrent.dirty() {
                torrent.serialize();
            }
        }
    }
}

pub struct TorrentTxUpdate {
    speeds: HashMap<usize, Speed>,
}

impl TorrentTxUpdate {
    pub fn new() -> TorrentTxUpdate {
        TorrentTxUpdate { speeds: HashMap::new() }
    }
}

struct Speed {
    ul: u64,
    dl: u64,
    linger: u8,
}

impl<T: cio::CIO> Job<T> for TorrentTxUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (id, torrent) in torrents.iter_mut() {
            let (ul, dl) = torrent.get_last_tx_rate();
            if !self.speeds.contains_key(id) {
                self.speeds.insert(
                    *id,
                    Speed {
                        dl: 0,
                        ul: 0,
                        linger: 0,
                    },
                );
            }
            let ls = self.speeds.get_mut(id).unwrap();
            // TODO: Use this result to get a better estimate
            if ls.ul != ul || ls.dl != dl || ls.linger != 0 {
                torrent.update_rpc_transfer();
                torrent.reset_last_tx_rate();
                ls.ul = ul;
                ls.dl = dl;
                if ls.linger == 0 {
                    ls.linger = 2;
                } else {
                    ls.linger -= 1;
                }
            }
        }
        self.speeds.retain(|id, _| torrents.contains_key(id));
    }
}

impl TorrentStatusUpdate {
    pub fn new() -> TorrentStatusUpdate {
        TorrentStatusUpdate { transferred: HashMap::new() }
    }
}

pub struct TorrentStatusUpdate {
    transferred: HashMap<usize, (u64, u64)>,
}

impl<T: cio::CIO> Job<T> for TorrentStatusUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (id, torrent) in torrents.iter_mut() {
            let (ul, dl) = (torrent.uploaded(), torrent.downloaded());
            if !self.transferred.contains_key(id) {
                self.transferred.insert(*id, (
                    torrent.uploaded(),
                    torrent.downloaded(),
                ));
            }
            let tx = self.transferred.get_mut(id).unwrap();
            if torrent.status() == Status::Seeding && ul == tx.0 {
                torrent.set_status(Status::Idle);
            }
            if torrent.status() == Status::Leeching && dl == tx.1 {
                torrent.set_status(Status::Pending);
            }
            tx.0 = ul;
            tx.1 = dl;
        }
        self.transferred.retain(|id, _| torrents.contains_key(id));
    }
}
