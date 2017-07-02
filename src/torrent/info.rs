use bencode::BEncode;
use std::path::PathBuf;
use std::collections::BTreeMap;
use std::{io, fs, fmt, cmp};
use sha1::Sha1;
use util::torrent_name;
use disk;

#[derive(Serialize, Deserialize, Clone)]
pub struct Info {
    pub name: String,
    pub announce: String,
    pub piece_len: u32,
    pub total_len: u64,
    pub hashes: Vec<Vec<u8>>,
    pub hash: [u8; 20],
    pub files: Vec<File>,
    pub private: bool,
}

impl fmt::Debug for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Torrent Info {{ name: {:?}, announce: {:?}, piece_len: {:?}, total_len: {:?}, hash: {:?}, files: {:?} }}",
               self.name, self.announce, self.piece_len, self.total_len, torrent_name(&self.hash), self.files)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct File {
    pub path: PathBuf,
    pub length: usize,
}

impl File {
    fn from_bencode(data: BEncode) -> Result<File, &'static str> {
        let mut d = data.to_dict().ok_or("File must be a dictionary type!")?;
        match (d.remove("name"), d.remove("path"), d.remove("length")) {
            (Some(v), None, Some(l)) => {
                let f = File {
                    path: PathBuf::from(v.to_string().ok_or("Path must be a valid string.")?),
                    length: l.to_int().ok_or("File length must be a valid int")? as usize,
                };
                Ok(f)
            }
            (None, Some(path), Some(l)) => {
                let mut p = PathBuf::new();
                for dir in path.to_list().ok_or("File path should be a list")? {
                    p.push(dir.to_string().ok_or("File path parts should be strings")?);
                }
                let f = File {
                    path: p,
                    length: l.to_int().ok_or("File length must be a valid int")? as usize,
                };
                Ok(f)
            }
            _ => Err("File dict must contain length and name or path"),
        }
    }

    fn create(&self) -> Result<(), io::Error> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let f = fs::OpenOptions::new().write(true).create(true).open(&self.path)?;
        f.set_len(self.length as u64)?;
        Ok(())
    }
}

impl Info {
    pub fn from_bencode(data: BEncode) -> Result<Info, &'static str> {
        data.to_dict()
            .and_then(|mut d| d.remove("info").and_then(|i| i.to_dict()).map(|i| (d, i)))
            .ok_or("")
            .and_then(|(mut d, mut i)| {
                let mut m = Sha1::new();
                let mut info_bytes = Vec::new();
                // TODO: Deal with this error/maybe convert everything to io::Error
                BEncode::Dict(i.clone()).encode(&mut info_bytes).unwrap();
                m.update(&info_bytes);
                let hash = m.digest().bytes();

                let a = d.remove("announce")
                    .and_then(|a| a.to_string())
                    .ok_or("Info must have announce url")?;
                let pl = i.remove("piece length")
                    .and_then(|i| i.to_int())
                    .ok_or("Info must specify piece length")?;
                let hashes = i.remove("pieces")
                    .and_then(|p| p.to_bytes())
                    .map(|mut p| {
                        let mut v = Vec::new();
                        while !p.is_empty() {
                            let remaining = p.split_off(20);
                            v.push(p);
                            p = remaining;
                        }
                        v
                    })
                .ok_or("Info must provide valid hashes")?;

                let private = i.remove("private")
                    .and_then(|v| v.to_int())
                    .map(|p| p == 1)
                    .unwrap_or(false);

                let files = parse_bencode_files(i)?;
                let name = if files.is_empty() {
                    files[0].path.clone().into_os_string().into_string().unwrap()
                } else if !files[0].path.has_root() {
                    let mut piter = files[0].path.components();
                    piter.next().unwrap().as_os_str().to_os_string().into_string().unwrap()
                } else {
                    unreachable!();
                };

                let total_len = files.iter().map(|f| f.length as u64).sum();
                Ok(Info {
                    name,
                    announce: a,
                    piece_len: pl as u32,
                    hashes,
                    hash,
                    files,
                    total_len,
                    private,
                })
            })

    }

    #[cfg(test)]
    pub fn with_pieces(pieces: usize) -> Info {
        Info {
            name: String::from(""),
            announce: String::from(""),
            piece_len: 16384,
            total_len: 16384 * pieces as u64,
            hashes: vec![vec![0u8]; pieces],
            hash: [0u8; 20],
            files: vec![],
            private: false,
        }
    }
    pub fn create_files(&self) -> Result<(), io::Error> {
        for file in self.files.iter() {
            file.create()?;
        }
        Ok(())
    }

    pub fn block_len(&self, idx: u32, offset: u32) -> u32 {
        if idx != self.pieces() - 1 {
            16384
        } else {
            let last_piece_len = (self.total_len - self.piece_len as u64 * (self.pieces() as u64 - 1)) as u32;
            // Note this is not the real last block len, just what it will be IF the offset really
            // is for the last block
            let last_block_len = last_piece_len - offset;
            if offset < last_piece_len && last_block_len <= 16384 {
                last_block_len
            } else {
                16384
            }
        }
    }

    pub fn piece_len(&self, idx: u32) -> u32 {
        if idx != self.pieces() - 1 {
            self.piece_len
        } else {
            (self.total_len - self.piece_len as u64 * (self.pieces() as u64 - 1)) as u32
        }
    }

    pub fn pieces(&self) -> u32 {
        self.hashes.len() as u32
    }

    /// Calculates the file offsets for a given block at index/begin
    pub fn block_disk_locs(&self, index: u32, begin: u32) -> Vec<disk::Location> {
        let len = self.block_len(index, begin);
        self.calc_disk_locs(index, begin, len)
    }

    /// Calculates the file offsets for a given piece at index
    pub fn piece_disk_locs(&self, index: u32) -> Vec<disk::Location> {
        let len = self.piece_len(index);
        self.calc_disk_locs(index, 0, len)
    }

    /// Calculates the file offsets for a given index, begin, and block length.
    fn calc_disk_locs(&self, index: u32, begin: u32, mut len: u32) -> Vec<disk::Location> {
        // The absolute byte offset where we start processing data.
        let mut cur_start = index * self.piece_len as u32 + begin;
        // Current index of the data block we're writing
        let mut data_start = 0;
        // The current file end length.
        let mut fidx = 0;
        // Iterate over all file lengths, if we find any which end a bigger
        // idx than cur_start, write from cur_start..cur_start + file_write_len for that file
        // and continue if we're now at the end of the file.
        let mut locs = Vec::new();
        for f in self.files.iter() {
            fidx += f.length;
            if (cur_start as usize) < fidx {
                let file_write_len = cmp::min(fidx - cur_start as usize, len as usize);
                let offset = (cur_start - (fidx - f.length) as u32) as u64;
                if file_write_len == len as usize {
                    // The file is longer than our len, just write to it,
                    // exit loop
                    locs.push(disk::Location::new(f.path.clone(), offset, data_start, data_start + file_write_len));
                    break;
                } else {
                    // Write to the end of file, continue
                    locs.push(disk::Location::new(f.path.clone(), offset, data_start, data_start + file_write_len as usize));
                    len -= file_write_len as u32;
                    cur_start += file_write_len as u32;
                    data_start += file_write_len;
                }
            }
        }
        locs
    }
}

fn parse_bencode_files(mut data: BTreeMap<String, BEncode>) -> Result<Vec<File>, &'static str> {
    match data.remove("files").and_then(|l| l.to_list()) {
        Some(fs) => {
            let mut path = PathBuf::new();
            path.push(data.remove("name")
                      .and_then(|v| v.to_string())
                      .ok_or("Multifile mode must have a name field")?);
            let mut files = Vec::new();
            for f in fs {
                let mut file = File::from_bencode(f)?;
                file.path = path.join(file.path);
                files.push(file);
            }
            Ok(files)
        }
        None => File::from_bencode(BEncode::Dict(data)).map(|f| vec![f]),
    }
}
