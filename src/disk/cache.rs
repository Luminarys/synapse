use std::{fs, io, path};
#[cfg(target_pointer_width = "32")]
use std::io::{Read, Seek, SeekFrom, Write};

use memmap::MmapMut;
#[cfg(target_pointer_width = "32")]
use memmap::MmapOptions;

use CONFIG;
use util::{native, MHashMap};

/// Holds a file and mmap cache. Because 32 bit systems
/// can't mmap large files, we load them as needed.
pub struct FileCache {
    #[cfg(target_pointer_width = "32")]
    files: MHashMap<path::PathBuf, fs::File>,
    #[cfg(target_pointer_width = "32")]
    fallback: MmapMut,
    #[cfg(target_pointer_width = "64")]
    files: MHashMap<path::PathBuf, (fs::File, MmapMut)>,
}

impl FileCache {
    pub fn new() -> FileCache {
        FileCache {
            files: MHashMap::default(),
            #[cfg(target_pointer_width = "32")]
            fallback: MmapMut::map_anon(16_384).expect("mmap failed!"),
        }
    }

    pub fn get_file_range<R, F: FnMut(&mut [u8]) -> R>(
        &mut self,
        path: &path::Path,
        size: Option<u64>,
        offset: u64,
        len: usize,
        _read: bool,
        mut f: F,
    ) -> io::Result<R> {
        self.ensure_exists(path, size)?;

        #[cfg(target_pointer_width = "32")]
        {
            let file = self.files.get_mut(path).unwrap();
            // TODO: Consider more portable solution based on setting _FILE_OFFSET_BITS=64 or
            // mmap64 rather than this.
            if offset < ::std::usize::MAX as u64 {
                let mut mmap = MmapOptions::new().offset(offset as usize).len(len).map_anon()?;
                Ok(f(&mut *mmap))
            } else {
                file.seek(SeekFrom::Start(offset))?;
                let data = &mut self.fallback[0..len];
                if _read {
                    file.read_exact(data)?;
                }
                let res = Ok(f(data));
                if !_read {
                    file.write_all(&data)?;
                }
                res
            }
        }

        #[cfg(target_pointer_width = "64")]
        {
            Ok(
                f(
                    &mut self.files.get_mut(path).unwrap().1
                        [offset as usize..offset as usize + len],
                ),
            )
        }
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        #[cfg(target_pointer_width = "32")]
        self.files.remove(path);
        #[cfg(target_pointer_width = "64")]
        self.files.remove(path).map(|f| f.1.flush_async().ok());
    }

    pub fn flush_file(&mut self, path: &path::Path) {
        #[cfg(target_pointer_width = "32")]
        {
            self.files.get_mut(path).map(|f| f.sync_all().ok());
        }
        #[cfg(target_pointer_width = "64")]
        {
            self.files.get_mut(path).map(|f| f.1.flush_async().ok());
        }
    }

    fn ensure_exists(&mut self, path: &path::Path, len: Option<u64>) -> io::Result<()> {
        if !self.files.contains_key(path) {
            if self.files.len() >= CONFIG.net.max_open_files {
                let removal = self.files.iter().map(|(id, _)| id.clone()).next().unwrap();
                self.remove_file(&removal);
            }

            fs::create_dir_all(path.parent().unwrap())?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create(len.is_some())
                .read(true)
                .open(path)?;

            if len.is_some() && file.metadata()?.len() != len.unwrap() {
                native::fallocate(&file, len.unwrap())?;
            }

            #[cfg(target_pointer_width = "32")]
            self.files.insert(path.to_path_buf(), file);

            #[cfg(target_pointer_width = "64")]
            {
                let mmap = unsafe { MmapMut::map_mut(&file)? };
                self.files.insert(path.to_path_buf(), (file, mmap));
            }
        }
        Ok(())
    }
}

impl Drop for FileCache {
    fn drop(&mut self) {
        #[cfg(target_pointer_width = "32")]
        {
            for (_, file) in self.files.drain() {
                file.sync_all().ok();
            }
        }
        #[cfg(target_pointer_width = "64")]
        {
            for (_, (_, mmap)) in self.files.drain() {
                mmap.flush().ok();
            }
        }
    }
}
