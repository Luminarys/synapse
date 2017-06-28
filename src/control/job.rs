use torrent::Torrent;
use std::collections::HashMap;
use std::time;

pub trait Job {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent>);
}

pub struct JobManager {
    jobs: Vec<JobData>,
}

struct JobData {
    job: Box<Job>,
    last_updated: time::Instant,
    interval: time::Duration,
}

impl JobManager {
    pub fn new() -> JobManager {
        JobManager { jobs: Vec::new() }
    }

    pub fn add_job<T: Job + 'static>(&mut self, job: T, interval: time::Duration) {
        self.jobs.push(JobData { job: Box::new(job), interval, last_updated: time::Instant::now()})
    }

    pub fn update(&mut self, torrents: &mut HashMap<usize, Torrent>) {
        for j in self.jobs.iter_mut() {
            if j.last_updated.elapsed() > j.interval {
                j.job.update(torrents);
                j.last_updated = time::Instant::now();
            }
        }
    }
}

pub struct TrackerUpdate;

impl Job for TrackerUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_tracker();
        }
    }
}

pub struct UnchokeUpdate;

impl Job for UnchokeUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_unchoked();
        }
    }
}

pub struct SessionUpdate;

impl Job for SessionUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent>) {
        for (_, torrent) in torrents.iter_mut() {
            if torrent.dirty() {
                torrent.serialize();
            }
        }
    }
}

pub struct ReapPeers;

impl Job for ReapPeers {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.reap_peers();
        }
    }
}
