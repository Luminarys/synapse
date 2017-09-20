use std::{fs, path, io};
use std::collections::HashMap;

use CONFIG;

pub struct FileCache {
    files: HashMap<path::PathBuf, fs::File>,
}

impl FileCache {
    pub fn new() -> FileCache {
        FileCache { files: HashMap::new() }
    }

    pub fn get_file<R, F: FnMut(&mut fs::File) -> io::Result<R>>(
        &mut self,
        path: &path::Path,
        mut f: F,
    ) -> io::Result<R> {
        let mut res = None;
        let hit = if let Some(file) = self.files.get_mut(path) {
            res = Some(f(file)?);
            true
        } else {
            false
        };
        if !hit {
            // TODO: LRU maybe?
            if self.files.len() >= CONFIG.net.max_open_files {
                let removal = self.files.iter().map(|(id, _)| id.clone()).next().unwrap();
                self.files.remove(&removal);
            }
            fs::create_dir_all(path.parent().unwrap())?;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .read(true)
                .open(path)?;
            res = Some(f(&mut file)?);
            self.files.insert(path.to_path_buf(), file);
        }
        Ok(res.unwrap())
    }

    pub fn remove_file(&mut self, path: &path::Path) {
        self.files.remove(path);
    }
}
