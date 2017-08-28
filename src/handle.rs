use std::{io, thread};
use std::sync::atomic;
use std::fmt::Debug;
use amy;
use TC;

pub struct Handle<I, O> {
    pub tx: amy::Sender<O>,
    pub rx: amy::Receiver<I>,
}

impl<I: Debug + Send + 'static, O: Debug + Send + 'static> Handle<I, O> {
    pub fn new(
        creg: &mut amy::Registrar,
        hreg: &mut amy::Registrar,
    ) -> io::Result<(Handle<I, O>, Handle<O, I>)> {
        let (htx, crx) = hreg.channel::<O>()?;
        let (ctx, hrx) = creg.channel::<I>()?;
        let ch = Handle { tx: htx, rx: hrx };
        let hh = Handle { tx: ctx, rx: crx };
        Ok((ch, hh))
    }

    pub fn run<F: FnOnce(Handle<I, O>) + Send + 'static>(self, thread: &'static str, f: F) {
        thread::spawn(move || {
            TC.fetch_add(1, atomic::Ordering::SeqCst);
            debug!("{} thread started", thread);
            f(self);
            debug!("{} thread completed", thread);
            TC.fetch_sub(1, atomic::Ordering::SeqCst);
        });
    }

    pub fn send(&self, msg: O) -> Result<(), ()> {
        match self.tx.send(msg) {
            Ok(()) => Ok(()),
            _ => Err(()),
        }
    }

    pub fn recv(&mut self) -> Result<I, ()> {
        match self.rx.try_recv() {
            Ok(msg) => Ok(msg),
            _ => Err(()),
        }
    }
}
