use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::sync::atomic;

const MAX_BUFS: usize = 4096;
static BUF_COUNT: atomic::AtomicUsize = atomic::AtomicUsize::new(0);

#[derive(Clone)]
pub struct Buffer {
    data: Box<MaybeUninit<[u8; 16_384]>>,
}

impl Buffer {
    pub fn get() -> Option<Buffer> {
        if BUF_COUNT.load(atomic::Ordering::Acquire) >= MAX_BUFS && !cfg!(test) {
            return None;
        }
        BUF_COUNT.fetch_add(1, atomic::Ordering::AcqRel);
        Some(Buffer {
            data: Box::new(MaybeUninit::uninit()),
        })
    }
}

impl Deref for Buffer {
    type Target = [u8; 16_384];

    fn deref(&self) -> &Self::Target {
        unsafe { &*self.data.as_ptr() }
    }
}

impl DerefMut for Buffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.data.as_mut_ptr() }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        BUF_COUNT.fetch_sub(1, atomic::Ordering::AcqRel);
    }
}
