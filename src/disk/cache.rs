use std::ffi::OsString;
use std::{fs, io, mem, path};

use std::os::unix::fs::MetadataExt;

#[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
use std::io::{Read, Seek, SeekFrom, Write};

#[cfg(all(feature = "mmap", target_pointer_width = "64"))]
use memmap::MmapMut;

#[cfg(all(feature = "mmap", target_pointer_width = "64"))]
use util::io_err;
use util::{native, MHashMap};
use CONFIG;

const PB_LEN: usize = 256;

pub struct BufCache {
    path_a: OsString,
    path_b: OsString,
    buf: Vec<u8>,
}

/// Holds a file and mmap cache. Because 32 bit systems
/// can't mmap large files, we load them as needed.
pub struct FileCache {
    files: MHashMap<path::PathBuf, Entry>,
}

#[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
pub struct Entry {
    used: bool,
    alloc_failed: bool,
    sparse: bool,
    file: fs::File,
}

#[cfg(all(feature = "mmap", target_pointer_width = "64"))]
pub struct Entry {
    mmap: MmapMut,
    used: bool,
    alloc_failed: bool,
    sparse: bool,
}

pub struct TempPB<'a> {
    path: path::PathBuf,
    buf: &'a mut OsString,
}

pub struct TempBuf<'a> {
    buf: &'a mut Vec<u8>,
}

impl<'a> TempBuf<'a> {
    pub fn get(&mut self, len: usize) -> &mut [u8] {
        unsafe {
            self.buf.reserve(len);
            self.buf.set_len(len);
        }
        &mut self.buf[..]
    }
}

fn get_pb(buf: &mut OsString) -> TempPB {
    debug_assert!(buf.capacity() >= PB_LEN);
    let path = mem::replace(buf, OsString::with_capacity(0)).into();
    TempPB { buf, path }
}

impl<'a> TempPB<'a> {
    pub fn get<P: AsRef<path::Path>>(&mut self, base: P) -> &mut path::PathBuf {
        self.clear();
        self.path.push(base.as_ref());
        &mut self.path
    }

    fn clear(&mut self) {
        let mut s =
            mem::replace(&mut self.path, OsString::with_capacity(0).into()).into_os_string();
        s.clear();
        self.path = s.into();
    }
}

impl<'a> Drop for TempPB<'a> {
    fn drop(&mut self) {
        let mut path =
            mem::replace(&mut self.path, OsString::with_capacity(0).into()).into_os_string();
        mem::swap(self.buf, &mut path);
        self.buf.clear();
    }
}

impl BufCache {
    pub fn new() -> BufCache {
        BufCache {
            path_a: OsString::with_capacity(PB_LEN),
            path_b: OsString::with_capacity(PB_LEN),
            buf: Vec::with_capacity(1_048_576),
        }
    }

    pub fn data(&mut self) -> (TempBuf, TempPB, TempPB) {
        (
            TempBuf { buf: &mut self.buf },
            get_pb(&mut self.path_a),
            get_pb(&mut self.path_b),
        )
    }
}

impl FileCache {
    pub fn new() -> FileCache {
        FileCache {
            files: MHashMap::default(),
        }
    }

    pub fn read_file_range(
        &mut self,
        path: &path::Path,
        offset: u64,
        buf: &mut [u8],
    ) -> io::Result<()> {
        self.ensure_exists(path, Err(0))?;

        #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
        {
            let entry = self.files.get_mut(path).unwrap();
            entry.file.seek(SeekFrom::Start(offset))?;
            entry.file.read_exact(buf)?;
            Ok(())
        }

        #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
        {
            let len = buf.len();
            let entry = self.files.get_mut(path).unwrap();
            entry.used = true;
            if entry.sparse {
                let res = native::mmap_read(
                    &entry.mmap[offset as usize..offset as usize + len],
                    buf,
                    len,
                );
                if res.is_err() {
                    return io_err("Disk full!");
                }
            } else {
                buf.copy_from_slice(&entry.mmap[offset as usize..offset as usize + len]);
            };
            Ok(())
        }
    }

    pub fn write_file_range(
        &mut self,
        path: &path::Path,
        size: Result<u64, u64>,
        offset: u64,
        buf: &[u8],
    ) -> io::Result<()> {
        self.ensure_exists(path, size)?;

        #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
        {
            let entry = self.files.get_mut(path).unwrap();
            entry.file.seek(SeekFrom::Start(offset))?;
            entry.file.write_all(&buf)?;
            Ok(())
        }

        #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
        {
            let len = buf.len();
            let entry = self.files.get_mut(path).unwrap();
            entry.used = true;
            if entry.sparse {
                let res = native::mmap_write(
                    &mut entry.mmap[offset as usize..offset as usize + len],
                    &buf,
                    len,
                );
                if res.is_err() {
                    return io_err("Disk full!");
                }
            } else {
                (&mut entry.mmap[offset as usize..offset as usize + len]).copy_from_slice(&buf);
            };
            Ok(())
        }
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
        self.files.remove(path);
        #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
        self.files.remove(path).map(|f| f.mmap.flush_async().ok());
    }

    pub fn flush_file(&mut self, path: &path::Path) {
        #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
        {
            self.files.get_mut(path).map(|e| e.file.sync_all().ok());
        }
        #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
        {
            self.files.get_mut(path).map(|f| f.mmap.flush_async().ok());
        }
    }

    fn ensure_exists(&mut self, path: &path::Path, len: Result<u64, u64>) -> io::Result<()> {
        let len_val = if let Ok(v) = len {
            v
        } else {
            len.err().unwrap()
        };
        if !self.files.contains_key(path) {
            if self.files.len() >= CONFIG.net.max_open_files {
                let mut removal = None;
                // We rely on random iteration order to prove us something close to a "clock hand"
                // like algorithm
                for (id, entry) in &mut self.files {
                    if entry.used {
                        entry.used = false;
                    } else {
                        removal = Some(id.clone());
                    }
                }
                if let Some(f) = removal {
                    self.remove_file(&f);
                }
            }

            fs::create_dir_all(path.parent().unwrap())?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(path)?;

            let alloc_failed = if len.is_ok() && file.metadata()?.len() != len.ok().unwrap() {
                let res = !native::fallocate(&file, len.unwrap())?;
                debug!("Attempted to fallocate {:?}: success {}!", path, !res);
                res
            } else {
                if len_val != 0 {
                    file.set_len(len_val)?;
                }
                false
            };

            let stat = file.metadata()?;
            let sparse = stat.blocks() * stat.blksize() < stat.size();

            #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
            self.files.insert(
                path.to_path_buf(),
                Entry {
                    file,
                    used: true,
                    sparse,
                    alloc_failed,
                },
            );

            #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
            {
                // Check if the file was never allocated
                if stat.size() == 0 {
                    return io_err("mmap attempted on 0 sized file");
                }
                let mmap = unsafe { MmapMut::map_mut(&file)? };
                self.files.insert(
                    path.to_path_buf(),
                    Entry {
                        mmap,
                        sparse,
                        used: true,
                        alloc_failed,
                    },
                );
            }
        } else if len.is_ok() {
            let entry = self.files.get_mut(path).unwrap();
            if entry.sparse && !entry.alloc_failed {
                debug!("Attempting delayed falloc!");
                let file = fs::OpenOptions::new().write(true).read(true).open(path)?;
                entry.alloc_failed = !native::fallocate(&file, len_val)?;
                if !entry.alloc_failed {
                    entry.sparse = false;
                }
            }
        }
        Ok(())
    }
}

impl Drop for FileCache {
    fn drop(&mut self) {
        #[cfg(any(target_pointer_width = "32", not(feature = "mmap")))]
        {
            for (_, entry) in self.files.drain() {
                entry.file.sync_all().ok();
            }
        }
        #[cfg(all(feature = "mmap", target_pointer_width = "64"))]
        {
            for (_, entry) in self.files.drain() {
                entry.mmap.flush().ok();
            }
        }
    }
}
