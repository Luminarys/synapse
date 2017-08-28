#![allow(unused_doc_comment)]
#![cfg_attr(feature="clippy", feature(plugin))]

extern crate amy;
extern crate byteorder;
extern crate rand;
extern crate ring;
extern crate url;
#[macro_use]
extern crate lazy_static;
extern crate net2;
extern crate serde;
extern crate serde_json;
#[macro_use]
extern crate serde_derive;
extern crate bincode;
extern crate toml;
extern crate signal;
#[macro_use]
extern crate error_chain;
extern crate c_ares;
extern crate httparse;
extern crate base64;
extern crate base32;
extern crate shellexpand;
extern crate chrono;

// TODO: Get rid of this
extern crate num;

#[macro_use]
mod log;

mod handle;
mod bencode;
mod torrent;
mod util;
mod socket;
mod disk;
mod tracker;
mod control;
mod listener;
mod rpc;
mod throttle;
mod config;

use std::{time, thread};
use std::sync::{atomic, mpsc};
use std::io;

use control::acio;
use log::LogLevel;

pub const DHT_EXT: (usize, u8) = (7, 1);

/// Throttler max token amount
pub const THROT_TOKS: usize = 2 * 1024 * 1024;

lazy_static! {
    pub static ref TC: atomic::AtomicUsize = {
        atomic::AtomicUsize::new(0)
    };

    pub static ref SHUTDOWN: atomic::AtomicBool = {
        atomic::AtomicBool::new(false)
    };

    pub static ref CONFIG: config::Config = {
        if let Ok(cfg)  = config::ConfigFile::try_load() {
            info!("Loaded config file");
            config::Config::from_file(cfg)
        } else {
            info!("Using default config");
            Default::default()
        }
    };

    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        let prefix = b"-SY0001-";
        for i in 0..prefix.len() {
            pid[i] = prefix[i];
        }

        let mut rng = rand::thread_rng();
        for i in prefix.len()..20 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };
}

fn init() -> io::Result<()> {
    let cpoll = amy::Poller::new()?;
    let mut creg = cpoll.get_registrar()?;
    let dh = disk::start(&mut creg)?;
    let lh = listener::Listener::start(&mut creg)?;
    let rh = rpc::RPC::start(&mut creg)?;
    let th = tracker::Tracker::start(&mut creg)?;
    let chans = acio::ACChans {
        disk_tx: dh.tx,
        disk_rx: dh.rx,
        rpc_tx: rh.tx,
        rpc_rx: rh.rx,
        trk_tx: th.tx,
        trk_rx: th.rx,
        lst_tx: lh.tx,
        lst_rx: lh.rx,
    };
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let throttler = throttle::Throttler::new(0, 0, THROT_TOKS, &creg);
        let acio = acio::ACIO::new(cpoll, creg, chans);
        match control::Control::new(acio, throttler) {
            Ok(mut c) => {
                tx.send(Ok(())).unwrap();
                c.run();
            }
            Err(e) => {
                tx.send(Err(e)).unwrap();
            }
        }
    });
    rx.recv().unwrap()
}

fn main() {
    if cfg!(debug_assertions) {
        log::log_init(log::LogLevel::Debug);
    } else {
        log::log_init(log::LogLevel::Error);
    }
    info!("Initializing!");
    if let Err(e) = init() {
        error!("Couldn't initialize synapse: {}", e);
        thread::sleep(time::Duration::from_millis(50));
        return;
    }

    info!("Initialized!");
    // Catch SIGINT, then shutdown
    let t = signal::trap::Trap::trap(&[2]);
    let mut i = time::Instant::now();
    loop {
        i += time::Duration::from_secs(1);
        if t.wait(i).is_some() {
            info!("Shutting down!");
            // TODO make this less hacky
            SHUTDOWN.store(true, atomic::Ordering::SeqCst);
            while TC.load(atomic::Ordering::SeqCst) != 0 {
                thread::sleep(time::Duration::from_secs(1));
            }
            info!("Shutdown complete!");
            // Let any residual logs flush
            thread::sleep(time::Duration::from_millis(50));
            break;
        }
        if TC.load(atomic::Ordering::SeqCst) == 0 {
            info!("Shutdown complete!");
            break;
        }
    }
}
