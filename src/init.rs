use std::os::unix::io::RawFd;
use std::sync::{atomic, mpsc};
use std::{io, process, thread};

use nix::sys::signal;
use nix::{fcntl, libc, unistd};

use crate::control::acio;
use crate::{args, control, disk, listener, log, rpc, throttle, tracker};
use crate::{CONFIG, SHUTDOWN, THROT_TOKS};

static mut PIPE: (RawFd, RawFd) = (-1, -1);

pub fn init(args: args::Args) -> Result<(), ()> {
    if let Some(level) = args.level {
        log::log_init(level);
    } else if cfg!(debug_assertions) {
        log::log_init(log::LogLevel::Debug);
    } else {
        log::log_init(log::LogLevel::Info);
    }

    info!("Initializing");

    // Since the config is lazy loaded, dereference now to check it.
    CONFIG.port;

    if let Err(e) = init_signals() {
        error!("Failed to initialize signal handlers: {}", e);
        return Err(());
    }
    Ok(())
}

pub fn run() -> Result<(), ()> {
    match init_threads() {
        Ok(threads) => {
            for thread in threads {
                if thread.join().is_err() {
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
    let mut creg = cpoll.get_registrar();
    let (dh, disk_broadcast, dhj) = disk::start(&mut creg)?;
    let (lh, lhj) = listener::Listener::start(&mut creg)?;
    let (rh, rhj) = rpc::RPC::start(&mut creg, disk_broadcast.clone())?;
    let (th, thj) = tracker::Tracker::start(&mut creg, disk_broadcast.clone())?;
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
    let cdb = disk_broadcast.clone();
    let chj = thread::Builder::new()
        .name("control".to_string())
        .spawn(move || {
            let throttler = throttle::Throttler::new(None, None, THROT_TOKS, &creg).unwrap();
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

fn init_signals() -> nix::Result<()> {
    unsafe {
        PIPE = unistd::pipe2(fcntl::OFlag::O_CLOEXEC)?;
        fcntl::fcntl(PIPE.1, fcntl::FcntlArg::F_SETFL(fcntl::OFlag::O_NONBLOCK))?;
    }

    let handler = signal::SigHandler::Handler(sig_handler);
    let sigset = signal::SigSet::empty();
    let sigflags = signal::SaFlags::SA_RESTART;
    let sigact = signal::SigAction::new(handler, sigflags, sigset);
    unsafe {
        signal::sigaction(signal::Signal::SIGINT, &sigact)?;
        signal::sigaction(signal::Signal::SIGTERM, &sigact)?;
        signal::sigaction(signal::Signal::SIGHUP, &sigact)?;
    }
    thread::Builder::new()
        .name("sighandler".to_string())
        .spawn(move || {
            let mut buf = [0u8];

            loop {
                loop {
                    match unsafe { unistd::read(PIPE.0, &mut buf[..]) } {
                        Ok(1) => break,
                        Ok(_) => error!("Signal handler error"),
                        Err(nix::Error::Sys(nix::errno::Errno::EINTR)) => {}
                        Err(e) => error!("Signal handler error {}", e),
                    }
                }
                if SHUTDOWN.load(atomic::Ordering::SeqCst) {
                    info!("Terminating process!");
                    process::abort();
                } else {
                    info!("Shutting down cleanly. Interrupt again to shut down immediately.");
                    SHUTDOWN.store(true, atomic::Ordering::SeqCst);
                }
            }
        })
        .expect("Coudln't spawn thread");
    Ok(())
}

extern "C" fn sig_handler(_: libc::c_int) {
    unsafe {
        unistd::write(PIPE.1, &[0u8]).ok();
    }
}
