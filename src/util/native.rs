use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::io;

use nix::libc::c_int;
use nix::errno::Errno;

use util::io::io_err;

#[link(name = "fallocate")]
extern "C" {
    fn native_fallocate(fd: c_int, len: u64) -> c_int;
}

pub fn fallocate(f: &File, len: u64) -> io::Result<()> {
    // We ignore the len here, if you actually have a u64 max, then you're kinda fucked either way.
    loop {
        match unsafe { native_fallocate(f.as_raw_fd(), len) } {
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
