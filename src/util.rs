use std::io;
use rand::{self, Rng};
use std::fmt::Write as FWrite;
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use std::net::{SocketAddr, Ipv4Addr, SocketAddrV4};

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
    if b.len() == 0 {
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

pub fn random_sample<A, T>(iter: A) -> Option<T>
where
    A: Iterator<Item = T>,
{
    let mut elem = None;
    let mut i = 1f64;
    let mut rng = rand::thread_rng();
    for new_item in iter {
        if rng.gen::<f64>() < (1f64 / i) {
            elem = Some(new_item);
        }
        i += 1.0;
    }
    elem
}

pub fn random_string(len: usize) -> String {
    rand::thread_rng()
        .gen_ascii_chars()
        .take(len)
        .collect::<String>()
}

pub fn torrent_name(hash: &[u8; 20]) -> String {
    let mut hash_str = String::new();
    for i in 0..20 {
        write!(&mut hash_str, "{:02X}", hash[i]).unwrap();
    }
    hash_str
}

pub fn bytes_to_addr(p: &[u8]) -> SocketAddr {
    let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
    SocketAddr::V4(SocketAddrV4::new(
        ip,
        (&p[4..]).read_u16::<BigEndian>().unwrap(),
    ))
}

pub fn addr_to_bytes(addr: &SocketAddr) -> [u8; 6] {
    let mut data = [0u8; 6];
    match *addr {
        SocketAddr::V4(s) => {
            let oct = s.ip().octets();
            data[0] = oct[0];
            data[1] = oct[1];
            data[2] = oct[2];
            data[3] = oct[3];
            (&mut data[4..]).write_u16::<BigEndian>(s.port()).unwrap();
        }
        _ => unimplemented!(),
    }
    data
}
