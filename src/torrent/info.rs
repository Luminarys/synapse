use bencode::BEncode;
use std::path::PathBuf;
use std::collections::BTreeMap;
use std::{io, fs};
use sha1::Sha1;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Info {
    pub name: String,
    pub announce: String,
    pub piece_len: usize,
    pub total_len: u64,
    pub hashes: Vec<Vec<u8>>,
    pub hash: [u8; 20],
    pub files: Vec<File>,
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
                        while p.len() != 0 {
                            let remaining = p.split_off(20);
                            v.push(p);
                            p = remaining;
                        }
                        v
                    })
                    .ok_or("Info must provide valid hashes")?;
                let files = parse_bencode_files(i)?;
                let name = if files.len() == 0 {
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
                    piece_len: pl as usize,
                    hashes,
                    hash,
                    files,
                    total_len,
                })
            })
    }

    pub fn create_files(&self) -> Result<(), io::Error> {
        for file in self.files.iter() {
            file.create()?;
        }
        Ok(())
    }

    pub fn last_piece_len(&self) -> u32 {
        let res = (self.total_len % 16384) as u32;
        if res == 0 {
            16384
        } else {
            res
        }
    }

    pub fn is_last_piece(&self, (idx, offset): (u32, u32)) -> bool {
        let last_piece_len = (self.total_len - self.piece_len as u64 * (self.pieces() as u64 - 1)) as u32;
        if offset < last_piece_len {
            let last_offset = last_piece_len - offset;
            idx == self.pieces() - 1 && last_offset <= 16384
        } else {
            false
        }
    }

    pub fn pieces(&self) -> u32 {
        self.hashes.len() as u32
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
            return Ok(files);
        }
        None => File::from_bencode(BEncode::Dict(data)).map(|f| vec![f]),
    }
}
