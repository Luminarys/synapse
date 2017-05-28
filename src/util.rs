use std::io;
use url::percent_encoding::{percent_encode_byte};

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
