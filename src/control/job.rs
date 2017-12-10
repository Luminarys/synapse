use std::time;

use torrent::Torrent;
use control::cio;
use util::UHashMap;

pub trait Job<T: cio::CIO> {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>);
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

    pub fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
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
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.try_update_tracker();
        }
    }
}

pub struct UnchokeUpdate;

impl<T: cio::CIO> Job<T> for UnchokeUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            torrent.update_unchoked();
        }
    }
}

pub struct SessionUpdate;

impl<T: cio::CIO> Job<T> for SessionUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            if torrent.dirty() {
                torrent.serialize();
            }
        }
    }
}

pub struct TorrentTxUpdate;

impl TorrentTxUpdate {
    pub fn new() -> TorrentTxUpdate {
        TorrentTxUpdate
    }
}

impl<T: cio::CIO> Job<T> for TorrentTxUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (_, torrent) in torrents.iter_mut() {
            if torrent.tick() {
                torrent.update_rpc_transfer();
            }
        }
    }
}

impl TorrentStatusUpdate {
    pub fn new() -> TorrentStatusUpdate {
        TorrentStatusUpdate { transferred: UHashMap::default() }
    }
}

pub struct TorrentStatusUpdate {
    transferred: UHashMap<(u64, u64)>,
}

impl<T: cio::CIO> Job<T> for TorrentStatusUpdate {
    fn update(&mut self, torrents: &mut UHashMap<Torrent<T>>) {
        for (id, torrent) in torrents.iter_mut() {
            let (ul, dl) = (torrent.uploaded(), torrent.downloaded());
            if !self.transferred.contains_key(id) {
                self.transferred.insert(*id, (
                    torrent.uploaded(),
                    torrent.downloaded(),
                ));
            }
            let tx = self.transferred.get_mut(id).unwrap();
            tx.0 = ul;
            tx.1 = dl;
        }
        self.transferred.retain(|id, _| torrents.contains_key(id));
    }
}
