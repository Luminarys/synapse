use std::{io, thread};
use std::fmt::Debug;
use amy;

pub struct Handle<I, O> {
    pub tx: amy::Sender<O>,
    pub rx: amy::Receiver<I>,
    pub reg: amy::Registrar,
}

impl<I: Debug + Send + 'static, O: Debug + Send + 'static> Handle<I, O> {
    pub fn new(
        creg: &mut amy::Registrar,
        hreg: &mut amy::Registrar,
    ) -> io::Result<(Handle<I, O>, Handle<O, I>)> {
        let (htx, crx) = hreg.channel::<O>()?;
        let (ctx, hrx) = creg.channel::<I>()?;
        let ch = Handle {
            tx: htx,
            rx: hrx,
            reg: creg.try_clone()?,
        };
        let hh = Handle {
            tx: ctx,
            rx: crx,
            reg: hreg.try_clone()?,
        };
        Ok((ch, hh))
    }

    pub fn run<F: FnOnce(Handle<I, O>) + Send + 'static>(
        self,
        thread: &'static str,
        f: F,
    ) -> io::Result<thread::JoinHandle<()>> {
        let builder = thread::Builder::new().name(thread.to_owned());
        builder.spawn(move || {
            debug!("{} thread started", thread);
            f(self);
            debug!("{} thread completed", thread);
        })
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
