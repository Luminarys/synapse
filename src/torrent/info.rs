use bencode::BEncode;
use std::path::PathBuf;
use std::collections::{HashMap, BTreeMap};
use std::{fmt, cmp};
use url::Url;
use ring::digest;
use util::hash_to_id;
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
    pub file_idx: HashMap<PathBuf, usize>,
    pub private: bool,
    pub be_name: Option<Vec<u8>>,
}

impl fmt::Debug for Info {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "Torrent Info {{
                name: {:?},
                announce: {:?},
                piece_len: {:?},
                total_len: {:?},
                hash: {:?},
                files: {:?}
            }}",
            self.name,
            self.announce,
            self.piece_len,
            self.total_len,
            hash_to_id(&self.hash),
            self.files
        )
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct File {
    pub path: PathBuf,
    pub length: u64,
}

impl File {
    fn from_bencode(data: BEncode) -> Result<File, &'static str> {
        let mut d = data.into_dict().ok_or("File must be a dictionary type!")?;
        match (d.remove("name"), d.remove("path"), d.remove("length")) {
            (Some(v), None, Some(l)) => {
                let f = File {
                    path: PathBuf::from(v.into_string().ok_or("Path must be a valid string.")?),
                    length: l.into_int().ok_or("File length must be a valid int")? as u64,
                };
                Ok(f)
            }
            (None, Some(path), Some(l)) => {
                let mut p = PathBuf::new();
                for dir in path.into_list().ok_or("File path should be a list")? {
                    p.push(
                        dir.into_string().ok_or("File path parts should be strings")?,
                    );
                }
                let f = File {
                    path: p,
                    length: l.into_int().ok_or("File length must be a valid int")? as u64,
                };
                Ok(f)
            }
            _ => Err("File dict must contain length and name or path"),
        }
    }
}

impl Info {
    pub fn from_magnet(data: &str) -> Result<Info, &'static str> {
        let url = match Url::parse(data) {
            Ok(u) => u,
            Err(_) => return Err("Failed to parse magnet URL!"),
        };

        if url.scheme() != "magnet" {
            return Err("magnet URL must use magnet scheme");
        };
        // TODO: Actually implmeent this
        unimplemented!();
    }

    pub fn to_bencode(&self) -> BEncode {
        let mut info = BTreeMap::new();
        if let Some(ref n) = self.be_name {
            info.insert("name".to_owned(), BEncode::String(n.clone()));
        }
        if self.private {
            info.insert("private".to_owned(), BEncode::Int(1));
        }
        info.insert(
            "piece length".to_owned(),
            BEncode::Int(self.piece_len as i64),
        );
        let mut pieces = Vec::with_capacity(self.hashes.len() * 20);
        for h in &self.hashes {
            pieces.extend_from_slice(h);
        }
        info.insert("pieces".to_owned(), BEncode::String(pieces));
        if self.files.len() == 1 {
            info.insert(
                "length".to_owned(),
                BEncode::Int(self.files[0].length as i64),
            );
        } else {
            let files = self.files
                .iter()
                .map(|f| {
                    let mut fb = BTreeMap::new();
                    fb.insert("length".to_owned(), BEncode::Int(f.length as i64));
                    fb.insert(
                        "path".to_owned(),
                        BEncode::String(
                            f.path
                                .clone()
                                .into_os_string()
                                .into_string()
                                .unwrap()
                                .into_bytes(),
                        ),
                    );
                    BEncode::Dict(fb)
                })
                .collect();
            info.insert("files".to_owned(), BEncode::List(files));
        }
        BEncode::Dict(info)
    }

    pub fn from_bencode(data: BEncode) -> Result<Info, &'static str> {
        data.into_dict()
            .and_then(|mut d| {
                d.remove("info").and_then(|i| i.into_dict()).map(|i| (d, i))
            })
            .ok_or("invalid info field")
            .and_then(|(mut d, mut i)| {
                let mut info_bytes = Vec::new();
                BEncode::Dict(i.clone()).encode(&mut info_bytes).unwrap();
                let mut ctx = digest::Context::new(&digest::SHA1);
                ctx.update(&info_bytes[..]);
                let digest = ctx.finish();
                let mut hash = [0u8; 20];
                hash.copy_from_slice(digest.as_ref());

                let a = d.remove("announce").and_then(|a| a.into_string()).ok_or(
                    "Info must have announce url",
                )?;
                let pl = i.remove("piece length").and_then(|i| i.into_int()).ok_or(
                    "Info must specify piece length",
                )?;
                let hashes = i.remove("pieces")
                    .and_then(|p| p.into_bytes())
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

                let private = if let Some(v) = i.remove("private") {
                    v.into_int()
                        .and_then(|p| if p == 1 { Some(true) } else { None })
                        .ok_or("private key must be an integer equal to 1 if present!")?
                } else {
                    false
                };

                let be_name = if let Some(v) = i.get("name").cloned() {
                    Some(v.into_bytes().ok_or("name field must be a bitstring!")?)
                } else {
                    None
                };

                let files = parse_bencode_files(i)?;
                let name = if files.is_empty() {
                    files[0]
                        .path
                        .clone()
                        .into_os_string()
                        .into_string()
                        .map_err(|_| "Only UTF8 paths are accepted")?
                } else if !files[0].path.has_root() {
                    let mut piter = files[0].path.components();
                    piter
                        .next()
                        .unwrap()
                        .as_os_str()
                        .to_os_string()
                        .into_string()
                        .map_err(|_| "Only UTF8 paths are accepted")?
                } else {
                    unreachable!();
                };

                let mut file_idx = HashMap::new();
                for (i, file) in files.iter().enumerate() {
                    file_idx.insert(file.path.clone(), i);
                }
                let total_len = files.iter().map(|f| f.length).sum();
                Ok(Info {
                    name,
                    announce: a,
                    piece_len: pl as u32,
                    hashes,
                    hash,
                    files,
                    file_idx,
                    total_len,
                    private,
                    be_name,
                })
            })
    }

    #[cfg(test)]
    pub fn with_pieces(pieces: usize) -> Info {
        Info {
            name: String::from(""),
            announce: String::from(""),
            piece_len: 16_384,
            total_len: 16_384 * pieces as u64,
            hashes: vec![vec![0u8]; pieces],
            hash: [0u8; 20],
            files: vec![],
            file_idx: HashMap::new(),
            private: false,
            be_name: None,
        }
    }

    pub fn block_len(&self, idx: u32, offset: u32) -> u32 {
        if idx != self.pieces() - 1 {
            16_384
        } else {
            let last_piece_len =
                (self.total_len - self.piece_len as u64 * (self.pieces() as u64 - 1)) as u32;
            // Note this is not the real last block len, just what it will be IF the offset really
            // is for the last block
            let last_block_len = last_piece_len - offset;
            if offset < last_piece_len && last_block_len <= 16_384 {
                last_block_len
            } else {
                16_384
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
    fn calc_disk_locs(&self, index: u32, begin: u32, len: u32) -> Vec<disk::Location> {
        let mut len = len as u64;
        // The absolute byte offset where we start processing data.
        let mut cur_start = index as u64 * self.piece_len as u64 + begin as u64;
        // Current index of the data block we're writing
        let mut data_start = 0;
        // The current file end length.
        let mut fidx = 0;
        // Iterate over all file lengths, if we find any which end a bigger
        // idx than cur_start, write from cur_start..cur_start + file_write_len for that file
        // and continue if we're now at the end of the file.
        let mut locs = Vec::new();
        for f in &self.files {
            fidx += f.length;
            if cur_start < fidx {
                let file_write_len = cmp::min(fidx - cur_start, len);
                let offset = cur_start - (fidx - f.length);
                if file_write_len == len {
                    // The file is longer than our len, just write to it,
                    // exit loop
                    locs.push(disk::Location::new(
                        f.path.clone(),
                        offset,
                        data_start,
                        data_start + file_write_len,
                    ));
                    break;
                } else {
                    // Write to the end of file, continue
                    locs.push(disk::Location::new(
                        f.path.clone(),
                        offset,
                        data_start,
                        data_start + file_write_len,
                    ));
                    len -= file_write_len;
                    cur_start += file_write_len;
                    data_start += file_write_len;
                }
            }
        }
        locs
    }
}

fn parse_bencode_files(mut data: BTreeMap<String, BEncode>) -> Result<Vec<File>, &'static str> {
    match data.remove("files").and_then(|l| l.into_list()) {
        Some(fs) => {
            let mut path = PathBuf::new();
            path.push(data.remove("name").and_then(|v| v.into_string()).ok_or(
                "Multifile mode must have a name field",
            )?);
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
