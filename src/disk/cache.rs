use std::{fs, io, mem, path};
use std::ffi::OsString;

#[cfg(target_pointer_width = "64")]
use std::os::unix::fs::MetadataExt;

#[cfg(target_pointer_width = "32")]
use std::io::{Read, Seek, SeekFrom, Write};

use memmap::MmapMut;

#[cfg(target_pointer_width = "32")]
use memmap::MmapOptions;

use CONFIG;
use util::{io_err, native, MHashMap};

const PB_LEN: usize = 256;

pub struct BufCache {
    path_a: OsString,
    path_b: OsString,
    buf: Vec<u8>,
}

/// Holds a file and mmap cache. Because 32 bit systems
/// can't mmap large files, we load them as needed.
pub struct FileCache {
    #[cfg(target_pointer_width = "32")]
    fallback: MmapMut,
    files: MHashMap<path::PathBuf, Entry>,
}

#[cfg(target_pointer_width = "32")]
pub struct Entry {
    used: bool,
    file: fs::File,
}

#[cfg(target_pointer_width = "64")]
pub struct Entry {
    used: bool,
    mmap: MmapMut,
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
            buf: Vec::with_capacity(16777216),
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
            #[cfg(target_pointer_width = "32")]
            fallback: MmapMut::map_anon(16_384).expect("mmap failed!"),
        }
    }

    pub fn get_file_range(
        &mut self,
        path: &path::Path,
        size: Option<u64>,
        offset: u64,
        len: usize,
        read: bool,
        allocate: bool,
        buf: &mut [u8],
    ) -> io::Result<()> {
        self.ensure_exists(path, size, allocate)?;

        #[cfg(target_pointer_width = "32")]
        {
            let entry = self.files.get_mut(path).unwrap();
            if offset < ::std::usize::MAX as u64 {
                let mut mmap = MmapOptions::new()
                    .offset(offset as usize)
                    .len(len)
                    .map_anon()?;
                Ok(f(&mut *mmap))
            } else {
                entry.file.seek(SeekFrom::Start(offset))?;
                if read {
                    entry.file.read_exact(&mut buf)?;
                } else {
                    entry.file.write_all(&buf)?;
                }
                res
            }
        }

        #[cfg(target_pointer_width = "64")]
        {
            let entry = self.files.get_mut(path).unwrap();
            entry.used = true;
            let res = if entry.sparse {
                assert!(len == buf.len());
                let res = if read {
                    native::mmap_read(
                        &entry.mmap[offset as usize..offset as usize + len],
                        buf,
                        len,
                    )
                } else {
                    native::mmap_write(
                        &mut entry.mmap[offset as usize..offset as usize + len],
                        &buf,
                        len,
                    )
                };
                if res.is_err() {
                    return io_err("Disk full!");
                }
            } else {
                if read {
                    buf.copy_from_slice(&entry.mmap[offset as usize..offset as usize + len]);
                } else {
                    (&mut entry.mmap[offset as usize..offset as usize + len]).copy_from_slice(&buf);
                }
            };
            Ok(res)
        }
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        #[cfg(target_pointer_width = "32")]
        self.files.remove(path);
        #[cfg(target_pointer_width = "64")]
        self.files.remove(path).map(|f| f.mmap.flush_async().ok());
    }

    pub fn flush_file(&mut self, path: &path::Path) {
        #[cfg(target_pointer_width = "32")]
        {
            self.files.get_mut(path).map(|e| e.file.sync_all().ok());
        }
        #[cfg(target_pointer_width = "64")]
        {
            self.files.get_mut(path).map(|f| f.mmap.flush_async().ok());
        }
    }

    fn ensure_exists(
        &mut self,
        path: &path::Path,
        len: Option<u64>,
        allocate: bool,
    ) -> io::Result<()> {
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
                removal.map(|f| self.remove_file(&f));
            }

            fs::create_dir_all(path.parent().unwrap())?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create(len.is_some())
                .read(true)
                .open(path)?;

            let alloc_failed =
                if allocate && len.is_some() && file.metadata()?.len() != len.unwrap() {
                    let res = !native::fallocate(&file, len.unwrap())?;
                    debug!("Attempted to fallocate {:?}: success {}!", path, !res);
                    res
                } else {
                    if len.is_some() && !allocate {
                        file.set_len(len.unwrap())?;
                    }
                    false
                };

            #[cfg(target_pointer_width = "32")]
            self.files
                .insert(path.to_path_buf(), Entry { file, used: true });

            #[cfg(target_pointer_width = "64")]
            {
                let stat = file.metadata()?;
                // Check if the file was never allocated
                if stat.size() == 0 {
                    return io_err("mmap attempted on 0 sized file");
                }
                let sparse = stat.blocks() * stat.blksize() < stat.size();
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
        } else if len.is_some() {
            let entry = self.files.get_mut(path).unwrap();
            if entry.sparse && allocate && !entry.alloc_failed {
                debug!("Attempting delayed falloc!");
                let file = fs::OpenOptions::new().write(true).read(true).open(path)?;
                entry.alloc_failed = !native::fallocate(&file, len.unwrap())?;
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
        #[cfg(target_pointer_width = "32")]
        {
            for (_, entry) in self.files.drain() {
                entry.file.sync_all().ok();
            }
        }
        #[cfg(target_pointer_width = "64")]
        {
            for (_, entry) in self.files.drain() {
                entry.mmap.flush().ok();
            }
        }
    }
}
