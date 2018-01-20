use std::io;

pub fn io_err<T>(reason: &'static str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Other, reason))
}

pub fn io_err_val(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, reason)
}

/// IO Result type for working with
/// async IO
pub enum IOR {
    Complete,
    Incomplete(usize),
    Blocked,
    EOF,
    Err(io::Error),
}

/// Do an async read, returning the appropriate IOR.
pub fn aread<R: io::Read>(b: &mut [u8], r: &mut R) -> IOR {
    if b.is_empty() {
        return IOR::Complete;
    }
    match r.read(b) {
        Ok(0) => IOR::EOF,
        Ok(a) if a == b.len() => IOR::Complete,
        Ok(a) => IOR::Incomplete(a),
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => IOR::Blocked,
        Err(e) => IOR::Err(e),
    }
}

/// Do an async write, returning the appropriate IOR.
pub fn awrite<W: io::Write>(b: &[u8], w: &mut W) -> IOR {
    match w.write(b) {
        Ok(0) => IOR::EOF,
        Ok(a) if a == b.len() => IOR::Complete,
        Ok(a) => IOR::Incomplete(a),
        Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => IOR::Blocked,
        Err(e) => IOR::Err(e),
    }
}
