use config::Config;
use manager::{Manager, ManagerReq, ManagerResp};
use std::mem;
use std::thread;
use std::sync::mpsc;
use mio::channel;

pub struct TorrentParams {

}

pub struct TorrentUpdateParams {

}

pub struct StateParams {
}

pub enum Event {
}

pub struct Handle {
    manager_tx: channel::Sender<ManagerReq>,
    manager_rx: mpsc::Receiver<ManagerResp>,
    event_rx: mpsc::Receiver<Event>,
}

impl Handle {
    pub fn new(cores: usize) -> Handle {
        let (mtx, hrx) = mpsc::channel();
        let (htx, mrx) = channel::channel();
        let (etx, erx) = mpsc::channel();
        thread::spawn(move || {
            let mut manager = Manager::new(cores, mtx, mrx, etx);
            manager.run();
        });

        Handle {
            manager_tx: htx,
            manager_rx: hrx,
            event_rx: erx,
        }
    }

    pub fn get_torrent_info(&self, hash: String) {
        unimplemented!();
    }

    pub fn update_torrent(&mut self, params: TorrentUpdateParams) {
        unimplemented!();
    }

    pub fn add_torrent(&mut self, torrent: TorrentParams) {
        unimplemented!();
    }

    pub fn delete_torrent(&mut self, hash: String) {
        unimplemented!();
    }

    pub fn get_config(&self) -> Config {
        unimplemented!();
    }

    pub fn set_config(&mut self, settings: Config) {
        unimplemented!();
    }

    pub fn save_state(&self, params: StateParams) {
        unimplemented!();
    }

    pub fn restore_state(&mut self, params: StateParams) {
        unimplemented!();
    }

    pub fn get_events(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        while let Ok(e) = self.event_rx.try_recv() {
            events.push(e);
        }
        events
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.manager_tx.send(ManagerReq::Shutdown);
    }
}
