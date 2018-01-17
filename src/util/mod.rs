pub mod native;

use std::io;
use std::fmt::Write as FWrite;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::ops::Deref;
use std::hash::BuildHasherDefault;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use rand::{self, Rng};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use metrohash::MetroHash;
use sha1;
use fnv;

pub type FHashMap<K, V> = fnv::FnvHashMap<K, V>;
pub type FHashSet<T> = fnv::FnvHashSet<T>;
pub type UHashMap<T> = FHashMap<usize, T>;

pub type MBuildHasher = BuildHasherDefault<MetroHash>;
pub type MHashMap<K, V> = HashMap<K, V, MBuildHasher>;
pub type MHashSet<T> = HashSet<T, MBuildHasher>;
pub type SHashMap<T> = MHashMap<String, T>;

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

pub struct AView<T: 'static> {
    /// This is actually the Arc
    ptr: *const u8,
    val: &'static T,
}

impl<T> AView<T> {
    pub fn new<'a, U: 'static, F: FnOnce(&'a U) -> &'a T>(arc: &'a Arc<U>, f: F) -> AView<T> {
        let val_ptr = f(&*arc) as *const T;
        let val = unsafe { val_ptr.as_ref().unwrap() };
        let ptr = Arc::into_raw(arc.clone()) as *const u8;
        AView { val, ptr }
    }
}

impl<T> AsRef<T> for AView<T> {
    fn as_ref(&self) -> &T {
        self.val
    }
}

impl<T> Deref for AView<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.val
    }
}

impl<T> Drop for AView<T> {
    fn drop(&mut self) {
        unsafe {
            drop(Arc::from_raw(self.ptr));
        }
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

pub fn sha1_hash(data: &[u8]) -> [u8; 20] {
    let mut ctx = sha1::Sha1::new();
    ctx.update(data);
    ctx.digest().bytes()
}

pub fn peer_rpc_id(torrent: &[u8; 20], peer: u64) -> String {
    const PEER_ID: &'static [u8] = b"PEER";
    let mut idx = [0u8; 8];
    (&mut idx[..]).write_u64::<BigEndian>(peer).unwrap();

    let mut ctx = sha1::Sha1::new();
    ctx.update(torrent);
    ctx.update(PEER_ID);
    ctx.update(&idx[..]);
    hash_to_id(&ctx.digest().bytes())
}

pub fn file_rpc_id(torrent: &[u8; 20], file: &str) -> String {
    const FILE_ID: &'static [u8] = b"FILE";
    let mut ctx = sha1::Sha1::new();
    ctx.update(torrent);
    ctx.update(FILE_ID);
    ctx.update(file.as_bytes());
    hash_to_id(&ctx.digest().bytes())
}

pub fn trk_rpc_id(torrent: &[u8; 20], url: &str) -> String {
    const TRK_ID: &'static [u8] = b"TRK";
    let mut ctx = sha1::Sha1::new();
    ctx.update(torrent);
    ctx.update(TRK_ID);
    ctx.update(url.as_bytes());
    hash_to_id(&ctx.digest().bytes())
}

pub fn hash_to_id(hash: &[u8]) -> String {
    let mut hash_str = String::new();
    for i in hash {
        write!(&mut hash_str, "{:02X}", i).unwrap();
    }
    hash_str
}

pub fn id_to_hash(s: &str) -> Option<[u8; 20]> {
    let mut data = [0u8; 20];
    if s.len() != 40 {
        return None;
    }
    let mut c = s.chars();
    for i in &mut data {
        if let (Some(a), Some(b)) = (hex_to_bit(c.next().unwrap()), hex_to_bit(c.next().unwrap())) {
            *i = a << 4 | b
        } else {
            return None;
        }
    }
    Some(data)
}

fn hex_to_bit(c: char) -> Option<u8> {
    let r = match c {
        '0' => 0,
        '1' => 1,
        '2' => 2,
        '3' => 3,
        '4' => 4,
        '5' => 5,
        '6' => 6,
        '7' => 7,
        '8' => 8,
        '9' => 9,
        'a' | 'A' => 10,
        'b' | 'B' => 11,
        'c' | 'C' => 12,
        'd' | 'D' => 13,
        'e' | 'E' => 14,
        'f' | 'F' => 15,
        _ => return None,
    };
    Some(r)
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

pub fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[test]
fn test_hash_enc() {
    let hash = [8u8; 20];
    let s = hash_to_id(&hash);
    assert_eq!(id_to_hash(&s).unwrap(), hash);
}
