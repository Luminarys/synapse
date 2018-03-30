use std::sync::atomic;
use std::mem;
use std::ops::{Deref, DerefMut};

const MAX_BUFS: usize = 4096;
static BUF_COUNT: atomic::AtomicUsize = atomic::ATOMIC_USIZE_INIT;

#[derive(Clone)]
pub struct Buffer {
    data: Box<[u8; 16_384]>,
}

impl Buffer {
    pub fn get() -> Option<Buffer> {
        if BUF_COUNT.load(atomic::Ordering::Acquire) >= MAX_BUFS {
            return None;
        }
        BUF_COUNT.fetch_add(1, atomic::Ordering::AcqRel);
        unsafe {
            Some(Buffer {
                data: Box::new(mem::uninitialized()),
            })
        }
    }
}

impl Deref for Buffer {
    type Target = Box<[u8; 16_384]>;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        BUF_COUNT.fetch_sub(1, atomic::Ordering::AcqRel);
    }
}
