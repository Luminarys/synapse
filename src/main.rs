extern crate amy;
extern crate byteorder;
extern crate rand;
extern crate sha1;
extern crate url;
#[macro_use]
extern crate lazy_static;
extern crate net2;
extern crate serde;
extern crate serde_json;
extern crate tiny_http;
#[macro_use]
extern crate serde_derive;
extern crate bincode;
extern crate toml;
extern crate signal;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
extern crate threadpool;
#[macro_use]
extern crate error_chain;
extern crate c_ares;

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

use std::{time, env, thread};
use std::sync::atomic;
use std::io::Read;
use slog::Drain;

lazy_static! {
    pub static ref TC: atomic::AtomicUsize = {
        atomic::AtomicUsize::new(0)
    };

    pub static ref CONFIG: config::Config = {
        let args: Vec<_> = env::args().collect();
        if args.len() >= 2 {
            info!(LOG, "Using config file!");
            let mut s = String::new();
            let contents = std::fs::File::open(&args[1]).and_then(|mut f| {
                f.read_to_string(&mut s).map(|_| s)
            });
            match contents {
                Ok(s) => {
                    match toml::from_str(&s) {
                        Ok(cf) => config::Config::from_file(cf),
                        Err(e) => {
                            error!(LOG, "Failed to parse config: {}. Falling back to default.", e);
                            Default::default()
                        }
                    }
                }
                Err(e) => {
                    error!(LOG, "Failed to open config: {}. Falling back to default.", e);
                    Default::default()
                }
            }
        } else {
            info!(LOG, "Using default config");
            Default::default()
        }
    };

    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        let prefix = b"-SN0001-";
        for i in 0..prefix.len() {
            pid[i] = prefix[i];
        }

        let mut rng = rand::thread_rng();
        for i in 8..19 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };

    pub static ref CONTROL: control::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        let log = LOG.new(o!("thread" => "control"));
        control::start(log)
    };

    pub static ref DISK: disk::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        let log = LOG.new(o!("thread" => "disk"));
        disk::start(log)
    };


    pub static ref TRACKER: tracker::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        let log = LOG.new(o!("thread" => "tracker"));
        tracker::start(log)
    };

    pub static ref LISTENER: listener::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        let log = LOG.new(o!("thread" => "listener"));
        listener::start(log)
    };

    pub static ref RPC: rpc::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        let log = LOG.new(o!("thread" => "RPC"));
        rpc::start(log)
    };

    pub static ref LOG: slog::Logger = {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, o!())
    };
}

fn main() {
    info!(LOG, "Initializing!");

    CONFIG.port;
    LISTENER.init();
    RPC.init();
    DISK.init();
    TRACKER.init();

    info!(LOG, "Initialized!");
    // Catch SIGINT, then shutdown
    let t = signal::trap::Trap::trap(&[2]);
    let mut i = time::Instant::now();
    loop {
        i += time::Duration::from_secs(1);
        if t.wait(i).is_some() {
            info!(LOG, "Shutting down!");
            CONTROL.ctrl_tx.lock().unwrap().send(control::Request::Shutdown).unwrap();
            while TC.load(atomic::Ordering::SeqCst) != 0 {
                thread::sleep(time::Duration::from_secs(1));
            }
            info!(LOG, "Shutdown complete!");
            // Let any residual logs flush
            thread::sleep(time::Duration::from_millis(50));
            break;
        }
        if TC.load(atomic::Ordering::SeqCst) == 0 {
            info!(LOG, "Shutdown complete!");
            break;
        }
    }
}
