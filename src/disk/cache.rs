use std::{fs, path, io};

use memmap::{Mmap, Protection};

use CONFIG;
use util::MHashMap;

/// Holds a file and mmap cache. Because 32 bit systems
/// can't mmap large files, we load them as needed.
pub struct FileCache {
    #[cfg(target_pointer_width = "32")]
    files: MHashMap<path::PathBuf, fs::File>,
    #[cfg(target_pointer_width = "64")]
    files: MHashMap<path::PathBuf, (fs::File, Mmap)>,
}

impl FileCache {
    pub fn new() -> FileCache {
        FileCache { files: MHashMap::default() }
    }

    pub fn get_file<R, F: FnMut(&mut fs::File) -> io::Result<R>>(
        &mut self,
        path: &path::Path,
        mut f: F,
    ) -> io::Result<R> {
        self.ensure_exists(path)?;

        #[cfg(target_pointer_width = "32")]
        {
            f(self.files.get_mut(path).unwrap())
        }

        #[cfg(target_pointer_width = "64")]
        {
            f(&mut self.files.get_mut(path).unwrap().0)
        }
    }

    pub fn get_file_range<R, F: FnMut(&mut [u8]) -> R>(
        &mut self,
        path: &path::Path,
        offset: usize,
        len: usize,
        mut f: F,
    ) -> io::Result<R> {
        self.ensure_exists(path)?;

        #[cfg(target_pointer_width = "32")]
        {
            let file = f(self.files.get_mut(path).unwrap());
            let mmap = Mmap::open_with_offset(&file, Protection::ReadWrite, offset, len)?;
            Ok(f(unsafe { mmap.as_mut_slice() }))
        }

        #[cfg(target_pointer_width = "64")]
        {
            Ok(f(unsafe {
                &mut self.files.get_mut(path).unwrap().1.as_mut_slice()[offset..offset + len]
            }))
        }
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        self.files.remove(path);
    }

    fn ensure_exists(&mut self, path: &path::Path) -> io::Result<()> {
        if !self.files.contains_key(path) {
            if self.files.len() >= CONFIG.net.max_open_files {
                let removal = self.files.iter().map(|(id, _)| id.clone()).next().unwrap();
                self.files.remove(&removal);
            }
            fs::create_dir_all(path.parent().unwrap())?;
            let file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(path)?;

            #[cfg(target_pointer_width = "32")] self.files.insert(path.to_path_buf(), file);

            #[cfg(target_pointer_width = "64")]
            {
                let mmap = Mmap::open(&file, Protection::ReadWrite)?;
                self.files.insert(path.to_path_buf(), (file, mmap));
            }
        }
        Ok(())
    }
}
