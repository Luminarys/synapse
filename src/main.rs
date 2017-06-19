extern crate amy;
extern crate byteorder;
extern crate rand;
extern crate sha1;
extern crate url;
extern crate reqwest;
#[macro_use]
extern crate lazy_static;
extern crate pbr;
extern crate net2;
extern crate serde;
extern crate serde_json;
extern crate tiny_http;
#[macro_use]
extern crate serde_derive;
extern crate bincode;
extern crate toml;
extern crate signal;

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

lazy_static! {
    pub static ref TC: atomic::AtomicUsize = {
        atomic::AtomicUsize::new(0)
    };

    pub static ref CONFIG: util::Init<config::Config> = {
        util::Init::new()
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

    pub static ref DISK: disk::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        disk::start()
    };

    pub static ref CONTROL: control::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        control::start()
    };

    pub static ref TRACKER: tracker::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        tracker::start()
    };

    pub static ref LISTENER: listener::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        listener::start()
    };

    pub static ref RPC: rpc::Handle = {
        TC.fetch_add(1, atomic::Ordering::SeqCst);
        rpc::start()
    };
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let config = if args.len() >= 2 {
        let mut s = String::new();
        let mut f = std::fs::File::open(&args[1]).expect("Config file could not be opened!");
        f.read_to_string(&mut s).expect("Config file could not be read!");
        let cf = toml::from_str(&s).expect("Config file could not be parsed!");
        config::Config::from_file(cf)
    } else {
        Default::default()
    };
    LISTENER.init();
    RPC.init();
    CONFIG.set(config);
    &DISK.tx;

    // Catch SIGINT, then shutdown
    let t = signal::trap::Trap::trap(&[2]);
    let mut i = time::Instant::now();
    loop {
        i += time::Duration::from_secs(1);
        match t.wait(i) {
            Some(_) => {
                println!("Shutting down!");
                CONTROL.ctrl_tx.lock().unwrap().send(control::Request::Shutdown).unwrap();
                while TC.load(atomic::Ordering::SeqCst) != 0 {
                    thread::sleep(time::Duration::from_secs(1));
                }
                println!("Shut down!");
                break;
            }
            _ => { }
        }
    }
}
