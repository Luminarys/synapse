use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::io;

use nix::errno::Errno;
use nix::libc::c_void;

use util::io::io_err;

mod sys {
    use nix::libc::{c_int, c_void, size_t};

    #[link(name = "fallocate")]
    extern "C" {
        pub fn native_fallocate(fd: c_int, len: u64) -> c_int;
    }

    #[link(name = "mmap")]
    extern "C" {
        pub fn mmap_read(mmap: *const c_void, data: *mut c_void, len: size_t) -> c_int;
        pub fn mmap_write(mmap: *mut c_void, data: *const c_void, len: size_t) -> c_int;
    }
}

pub fn fallocate(f: &File, len: u64) -> io::Result<()> {
    // We ignore the len here, if you actually have a u64 max, then you're kinda fucked either way.
    loop {
        match unsafe { sys::native_fallocate(f.as_raw_fd(), len) } {
            0 => return Ok(()),
            -1 => match Errno::last() {
                Errno::EOPNOTSUPP | Errno::ENOSYS => {
                    f.set_len(len)?;
                    return Ok(());
                }
                Errno::ENOSPC => {
                    return io_err("Out of disk space!");
                }
                Errno::EINTR => {
                    continue;
                }
                e => {
                    return io_err(e.desc());
                }
            },
            _ => unreachable!(),
        }
    }
}

pub fn mmap_read(mmap: &[u8], data: &mut [u8], len: usize) -> Result<(), ()> {
    unsafe {
        if sys::mmap_read(
            mmap.as_ptr() as *const c_void,
            data.as_mut_ptr() as *mut c_void,
            len,
        ) == 0
        {
            Ok(())
        } else {
            Err(())
        }
    }
}

pub fn mmap_write(mmap: &mut [u8], data: &[u8], len: usize) -> Result<(), ()> {
    unsafe {
        if sys::mmap_write(
            mmap.as_mut_ptr() as *mut c_void,
            data.as_ptr() as *const c_void,
            len,
        ) == 0
        {
            Ok(())
        } else {
            Err(())
        }
    }
}
