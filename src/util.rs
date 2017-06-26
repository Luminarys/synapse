use std::io;
use rand::{self, Rng};
use url::percent_encoding::{percent_encode_byte};
use std::fmt::Write as FWrite;

pub fn io_err<T>(reason: &'static str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Other, reason))
}

pub fn io_err_val(reason: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::Other, reason)
}

pub fn append_pair(s: &mut String, k: &str, v: &str) {
    s.push_str(k);
    s.push_str("=");
    s.push_str(v);
    s.push_str("&");
}

pub fn encode_param(data: &[u8]) -> String {
    let mut resp = String::new();
    for byte in data {
        resp.push_str(percent_encode_byte(*byte));
    }
    resp
}

pub fn random_sample<A, T>(iter: A) -> Option<T>
    where A: Iterator<Item=T> {
    let mut elem = None;
    let mut i = 1f64;
    let mut rng = rand::thread_rng();
    for new_item in iter {
        if rng.gen::<f64>() < (1f64/i) {
            elem = Some(new_item);
        }
        i += 1.0;
    }
    elem
}

pub fn torrent_name(hash: &[u8; 20]) -> String {
    let mut hash_str = String::new();
    for i in 0..20 {
        write!(&mut hash_str, "{:X}", hash[i]).unwrap();
    }
    hash_str
}
