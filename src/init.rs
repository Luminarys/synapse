use std::{process, thread};
use std::sync::{atomic, mpsc};
use std::io;

use {amy, ctrlc};

use {args, control, disk, listener, log, rpc, throttle, tracker};
use {CONFIG, SHUTDOWN, THROT_TOKS};
use control::acio;

pub fn init(args: args::Args) {
    if let Some(level) = args.level {
        log::log_init(level);
    } else if cfg!(debug_assertions) {
        log::log_init(log::LogLevel::Debug);
    } else {
        log::log_init(log::LogLevel::Info);
    }

    info!("Initializing");

    // Since the config is lazy loaded, derefernce now to check it.
    CONFIG.port;

    ctrlc::set_handler(|| {
        if SHUTDOWN.load(atomic::Ordering::SeqCst) {
            info!("Shutting down immediately!");
            process::abort();
        } else {
            info!(
                "Caught SIGINT, shutting down cleanly. Interrupt again to shut down immediately."
            );
            SHUTDOWN.store(true, atomic::Ordering::SeqCst);
        }
    }).expect("Signal installation failed!");
}

pub fn run() -> Result<(), ()> {
    match init_threads() {
        Ok(threads) => {
            for thread in threads {
                if let Err(_) = thread.join() {
                    error!("Unclean shutdown detected, terminating");
                    return Err(());
                }
            }
            info!("Shutdown complete");
            Ok(())
        }
        Err(e) => {
            error!("Couldn't initialize synapse: {}", e);
            Err(())
        }
    }
}

fn init_threads() -> io::Result<Vec<thread::JoinHandle<()>>> {
    let cpoll = amy::Poller::new()?;
    let mut creg = cpoll.get_registrar()?;
    let (dh, disk_broadcast, dhj) = disk::start(&mut creg)?;
    let (lh, lhj) = listener::Listener::start(&mut creg)?;
    let (rh, rhj) = rpc::RPC::start(&mut creg, disk_broadcast.try_clone()?)?;
    let (th, thj) = tracker::Tracker::start(&mut creg, disk_broadcast.try_clone()?)?;
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
    let cdb = disk_broadcast.try_clone()?;
    let chj = thread::Builder::new()
        .name("control".to_string())
        .spawn(move || {
            let throttler = throttle::Throttler::new(None, None, THROT_TOKS, &creg);
            let acio = acio::ACIO::new(cpoll, creg, chans);
            match control::Control::new(acio, throttler, cdb) {
                Ok(mut c) => {
                    tx.send(Ok(())).unwrap();
                    c.run();
                }
                Err(e) => {
                    tx.send(Err(e)).unwrap();
                }
            }
        })
        .unwrap();
    rx.recv().unwrap()?;

    Ok(vec![chj, dhj, lhj, rhj, thj])
}
