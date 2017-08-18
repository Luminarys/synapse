#![allow(unused_doc_comment)]

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
extern crate slog;
extern crate slog_term;
extern crate slog_async;
#[macro_use]
extern crate error_chain;
extern crate c_ares;
extern crate httparse;
extern crate base64;
extern crate base32;

extern crate chrono;
// TODO: Get rid of this
extern crate num;

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

use std::{time, env, thread};
use std::sync::{atomic, mpsc};
use std::io::{self, Read};
use slog::Drain;
use control::acio;

pub const DHT_EXT: (usize, u8) = (7, 1);

/// Throttler max token amount
pub const THROT_TOKS: usize = 2 * 1024 * 1024;

pub const RAREST_PKR: bool = true;

lazy_static! {
    pub static ref TC: atomic::AtomicUsize = {
        atomic::AtomicUsize::new(0)
    };

    pub static ref SHUTDOWN: atomic::AtomicBool = {
        atomic::AtomicBool::new(false)
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
                        Ok(cf) => {
                            let mut c = config::Config::from_file(cf);
                            if !c.rpc.local && !c.rpc.auth {
                                error!(LOG, "Synapse must use authentication for a non local config!");
                                error!(LOG, "Overriding config to use local RPC!");
                                c.rpc.local = true
                            }
                            c
                        }
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

    pub static ref LOG: slog::Logger = {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, o!())
    };
}

fn init() -> io::Result<()> {
    let cpoll = amy::Poller::new()?;
    let mut creg = cpoll.get_registrar()?;
    let dh = disk::start(&mut creg)?;
    let lh = listener::start(&mut creg)?;
    let rh = rpc::RPC::start(&mut creg)?;
    let th = tracker::start(&mut creg)?;
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
        if let Ok(acio) = acio::ACIO::new(cpoll, creg, chans, LOG.new(o!("ctrl" => "acio"))) {
            if let Ok(mut ctrl) = control::Control::new(
                acio,
                throttler,
                LOG.new(o!("thread" => "ctrl")),
            )
            {
                tx.send(true).unwrap();
                ctrl.run();
            } else {
                tx.send(false).unwrap();
            }
        } else {
            tx.send(false).unwrap();
        }
    });
    if rx.recv().unwrap() {
        Ok(())
    } else {
        util::io_err("Failed to intialize control thread!")
    }
}

fn main() {
    info!(LOG, "Initializing!");
    if let Err(e) = init() {
        error!(LOG, "Couldn't initialize synapse: {}", e);
        thread::sleep(time::Duration::from_millis(50));
        return;
    }

    info!(LOG, "Initialized!");
    // Catch SIGINT, then shutdown
    let t = signal::trap::Trap::trap(&[2]);
    let mut i = time::Instant::now();
    loop {
        i += time::Duration::from_secs(1);
        if t.wait(i).is_some() {
            info!(LOG, "Shutting down!");
            // TODO make this less hacky
            SHUTDOWN.store(true, atomic::Ordering::SeqCst);
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
