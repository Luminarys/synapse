use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::AsRawFd;

use nix::errno::Errno;

use crate::util::io::io_err;

mod sys {
    use nix::libc::c_int;

    #[link(name = "fallocate")]
    extern "C" {
        pub fn native_fallocate(fd: c_int, len: u64) -> c_int;
    }
}

pub fn is_sparse(f: &File) -> io::Result<bool> {
    let stat = f.metadata()?;
    Ok(stat.blocks() * stat.blksize() < stat.size())
}

pub fn fallocate(f: &File, len: u64) -> io::Result<bool> {
    // We ignore the len here, if you actually have a u64 max, then you're kinda fucked either way.
    loop {
        match unsafe { sys::native_fallocate(f.as_raw_fd(), len) } {
            0 => return Ok(true),
            -1 => match Errno::last() {
                Errno::EOPNOTSUPP | Errno::ENOSYS => {
                    f.set_len(len)?;
                    return Ok(false);
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
