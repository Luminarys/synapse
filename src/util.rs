use std::{io, mem};
use std::cell::UnsafeCell;
use rand::{self, Rng};
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

pub struct Init<T>(UnsafeCell<Option<T>>);

impl<T> Init<T> {
    pub fn new() -> Init<T> {
        Init(UnsafeCell::new(None))
    }

    pub fn set(&self, val: T) {
        unsafe {
            let r = self.0.get().as_mut().unwrap();
            assert!(r.is_none());
            mem::replace(r, Some(val));
        }
    }

    pub fn get(&self) -> &T {
        unsafe {
            self.0.get().as_ref().unwrap().as_ref().unwrap()
        }
    }
}

unsafe impl<T> Send for Init<T> { }
unsafe impl<T> Sync for Init<T> { }
