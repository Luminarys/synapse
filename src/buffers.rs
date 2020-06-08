use std::ops::{Deref, DerefMut};
use std::sync::atomic;

use crate::protocol;

const MAX_BUFS: usize = 4096;
pub const BUF_SIZE: usize = 16_384;
static BUF_COUNT: atomic::AtomicUsize = atomic::AtomicUsize::new(0);

#[derive(Clone)]
pub struct Buffer {
    data: Box<[u8; BUF_SIZE]>,
}

impl Buffer {
    pub fn get() -> Option<Buffer> {
        if BUF_COUNT.load(atomic::Ordering::Acquire) >= MAX_BUFS && !cfg!(test) {
            return None;
        }
        BUF_COUNT.fetch_add(1, atomic::Ordering::AcqRel);
        Some(Buffer {
            data: Box::new([0; BUF_SIZE]),
        })
    }
}

impl Deref for Buffer {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &*self.data
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.data
    }
}

impl protocol::Buffer for Buffer {}

impl Drop for Buffer {
    fn drop(&mut self) {
        BUF_COUNT.fetch_sub(1, atomic::Ordering::AcqRel);
    }
}
