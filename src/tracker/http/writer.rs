use crate::tracker::errors::{ErrorKind, Result};
use std::io;

pub struct Writer {
    data: Vec<u8>,
    idx: usize,
}

impl Writer {
    pub fn new(data: Vec<u8>) -> Writer {
        Writer { data, idx: 0 }
    }

    pub fn writable<W: io::Write>(&mut self, conn: &mut W) -> Result<Option<()>> {
        match conn.write(&self.data[self.idx..]) {
            Ok(0) => Err(ErrorKind::EOF.into()),
            Ok(v) if self.idx + v == self.data.len() => Ok(Some(())),
            Ok(v) => {
                self.idx += v;
                Ok(None)
            }
            Err(e) => {
                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::NotConnected
                    // EPIPE can occur on WSL
                    || e.kind() == io::ErrorKind::BrokenPipe
                {
                    Ok(None)
                } else {
                    Err(ErrorKind::IO.into())
                }
            }
        }
    }
}
