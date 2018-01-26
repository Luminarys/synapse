use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::io;

use libc::c_int;

#[link(name = "fallocate")]
extern "C" {
    fn native_fallocate(fd: c_int, len: u64) -> c_int;
}

pub fn fallocate(f: &File, len: u64) -> io::Result<()> {
    // We ignore the len here, if you actually have a u64 max, then you're kinda fucked either way.
    match unsafe { native_fallocate(f.as_raw_fd(), len) } {
        0 => Ok(()),
        e => {
            error!("fallocate failed: {}", e);
            f.set_len(len)?;
            Ok(())
        }
    }
}
