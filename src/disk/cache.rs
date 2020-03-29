use std::ffi::OsString;
use std::{fs, io, mem, path};

use std::os::unix::fs::MetadataExt;

use std::io::{Read, Seek, SeekFrom, Write};

use util::{native, MHashMap};
use CONFIG;

const PB_LEN: usize = 256;

pub struct BufCache {
    path_a: OsString,
    path_b: OsString,
    buf: Vec<u8>,
}

pub struct FileCache {
    files: MHashMap<path::PathBuf, Entry>,
}

pub struct Entry {
    used: bool,
    alloc_failed: bool,
    sparse: bool,
    file: fs::File,
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
        self.buf.reserve(len);
        if self.buf.len() < len {
            self.buf.resize(len, 0u8);
        }
        &mut self.buf[..len]
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
        let entry = self.files.get_mut(path).unwrap();
        entry.file.seek(SeekFrom::Start(offset))?;
        entry.file.read_exact(buf)?;
        Ok(())
    }

    pub fn write_file_range(
        &mut self,
        path: &path::Path,
        size: Result<u64, u64>,
        offset: u64,
        buf: &[u8],
    ) -> io::Result<()> {
        self.ensure_exists(path, size)?;
        let entry = self.files.get_mut(path).unwrap();
        entry.file.seek(SeekFrom::Start(offset))?;
        entry.file.write_all(&buf)?;
        Ok(())
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        self.files.remove(path);
    }

    pub fn flush_file(&mut self, path: &path::Path) {
        self.files.get_mut(path).map(|e| e.file.sync_all().ok());
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

            self.files.insert(
                path.to_path_buf(),
                Entry {
                    file,
                    used: true,
                    sparse,
                    alloc_failed,
                },
            );
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
        for (_, entry) in self.files.drain() {
            entry.file.sync_all().ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tempbuf() {
        let mut data = vec![];
        let mut buf = TempBuf { buf: &mut data };
        assert_eq!(buf.get(10).len(), 10);
        assert_eq!(buf.get(20).len(), 20);
        assert_eq!(buf.get(10).len(), 10);
        assert_eq!(buf.get(30).len(), 30);
        assert_eq!(buf.get(10).len(), 10);
    }
}
