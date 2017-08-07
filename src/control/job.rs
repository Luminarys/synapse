use torrent::Torrent;
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
        self.jobs.push(JobData { job: Box::new(job), interval, last_updated: time::Instant::now()})
    }

    pub fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for j in self.jobs.iter_mut() {
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
            torrent.update_tracker();
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

pub struct TorrentTxUpdate;

impl<T: cio::CIO> Job<T> for TorrentTxUpdate {
    fn update(&mut self, torrents: &mut HashMap<usize, Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_rpc_transfer();
            // TODO: Use this result to get a better estimate
            torrent.reset_last_tx_rate();
        }
    }
}
