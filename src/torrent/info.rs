use std::path::PathBuf;
use std::collections::BTreeMap;
use std::{cmp, fmt, mem};
use std::sync::Arc;

use base32;
use url::Url;

use disk;
use bencode::BEncode;
use util::{hash_to_id, id_to_hash, sha1_hash};

#[derive(Clone)]
pub struct Info {
    pub name: String,
    pub announce: Option<Url>,
    pub piece_len: u32,
    pub total_len: u64,
    pub hashes: Vec<Vec<u8>>,
    pub hash: [u8; 20],
    pub files: Vec<File>,
    pub private: bool,
    pub be_name: Option<Vec<u8>>,
    pub piece_idx: Vec<(usize, u64)>,
    pub url_list: Vec<Vec<Url>>,
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

#[derive(Clone, Debug)]
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
                    p.push(dir.into_string()
                        .ok_or("File path parts should be strings")?);
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
            return Err("magnet URL must use magnet URL scheme");
        };
        let hash = url.query_pairs()
            .find(|&(ref k, ref v)| k == "xt" && v.starts_with("urn:btih:"))
            .and_then(|(_, ref v)| {
                id_to_hash(&v[9..]).or_else(|| {
                    base32::decode(base32::Alphabet::RFC4648 { padding: true }, &v[9..]).and_then(
                        |b| {
                            if b.len() != 20 {
                                return None;
                            }
                            let mut a = [0; 20];
                            (&mut a[..]).copy_from_slice(&b);
                            Some(a)
                        },
                    )
                })
            })
            .ok_or("No hash found in magnet")?;
        let announce = url.query_pairs()
            .find(|&(ref k, _)| k == "tr")
            .and_then(|(_, ref v)| Url::parse(v).map(|u| Some(u)).unwrap_or(None));
        let name = url.query_pairs()
            .find(|&(ref k, _)| k == "dn")
            .map(|(_, ref v)| v.to_string())
            .unwrap_or_else(|| "".to_owned());
        Ok(Info {
            name,
            announce,
            piece_len: 0,
            total_len: 0,
            hashes: vec![],
            hash,
            files: vec![],
            private: false,
            be_name: None,
            piece_idx: vec![],
            url_list: vec![],
        })
    }

    pub fn complete(&self) -> bool {
        !self.hashes.is_empty()
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
            BEncode::Int(i64::from(self.piece_len)),
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
            .and_then(|mut d| d.remove("info").and_then(|i| i.into_dict()).map(|i| (d, i)))
            .ok_or("invalid info field")
            .and_then(|(mut d, mut i)| {
                let mut info_bytes = Vec::new();
                BEncode::Dict(i.clone()).encode(&mut info_bytes).unwrap();
                let hash = sha1_hash(&info_bytes);

                let a = d.remove("announce")
                    .ok_or_else(|| "Info must have announce URL")?
                    .into_string()
                    .ok_or_else(|| "Info must have announce URL")
                    .and_then(|a| Url::parse(&a).map_err(|_| "Info has invalid announce URL"))?;
                let pl = i.remove("piece length")
                    .and_then(|i| i.into_int())
                    .ok_or("Info must specify piece length")? as u64;
                let hashes = i.remove("pieces")
                    .and_then(|p| p.into_bytes())
                    .and_then(|p| {
                        let mut v = Vec::new();
                        let mut s = &p[..];
                        while s.len() >= 20 {
                            let mut next = vec![0u8; 20];
                            next.clone_from_slice(&s[..20]);
                            v.push(next);
                            s = &s[20..];
                        }
                        if s.len() != 0 {
                            return None;
                        }
                        Some(v)
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
                    unreachable!()
                };

                let total_len = files.iter().map(|f| f.length).sum();
                let piece_idx = Info::generate_piece_idx(hashes.len(), pl, &files);

                let url_list: Vec<Vec<Url>> = d.remove("announce-list")
                    .and_then(BEncode::into_list)
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(|l| {
                        l.into_list()
                            .unwrap_or_else(Vec::new)
                            .into_iter()
                            .filter_map(BEncode::into_string)
                            .filter_map(|s| Url::parse(&s).ok())
                            .collect()
                    })
                    .collect();

                Ok(Info {
                    name,
                    announce: Some(a),
                    piece_len: pl as u32,
                    hashes,
                    hash,
                    files,
                    total_len,
                    private,
                    be_name,
                    piece_idx,
                    url_list,
                })
            })
    }

    pub fn generate_piece_idx(pieces: usize, pl: u64, files: &[File]) -> Vec<(usize, u64)> {
        let mut piece_idx = Vec::with_capacity(pieces);
        let mut file = 0;
        let mut offset = 0u64;
        for _ in 0..pieces {
            piece_idx.push((file, offset));
            offset += pl;
            while file < files.len() && offset >= files[file].length {
                offset -= files[file].length;
                file += 1;
            }
        }
        piece_idx
    }

    #[cfg(test)]
    pub fn with_pieces(pieces: usize) -> Info {
        Info {
            name: String::from(""),
            announce: None,
            piece_len: 16_384,
            total_len: 16_384 * pieces as u64,
            hashes: vec![vec![0u8]; pieces],
            hash: [0u8; 20],
            files: vec![
                File {
                    path: PathBuf::new(),
                    length: 16_384 * pieces as u64,
                };
                1
            ],
            private: false,
            be_name: None,
            piece_idx: vec![],
            url_list: vec![],
        }
    }

    #[cfg(test)]
    pub fn with_pieces_scale(pieces: u32, scale: u32) -> Info {
        Info {
            name: String::from(""),
            announce: None,
            piece_len: 16_384 * scale,
            total_len: 16_384 * pieces as u64 * scale as u64,
            hashes: vec![vec![0u8]; pieces as usize],
            hash: [0u8; 20],
            files: vec![],
            private: false,
            be_name: None,
            piece_idx: vec![],
            url_list: vec![],
        }
    }

    pub fn block_len(&self, idx: u32, offset: u32) -> u32 {
        if idx != self.pieces() - 1 {
            16_384
        } else {
            let last_piece_len = self.piece_len(idx);
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
        if !self.complete() {
            return 0;
        }
        if idx != self.pieces().saturating_sub(1) {
            self.piece_len
        } else {
            (self.total_len - u64::from(self.piece_len) * (u64::from(self.pieces()) - 1)) as u32
        }
    }

    pub fn pieces(&self) -> u32 {
        self.hashes.len() as u32
    }

    /// Calculates the file offsets for a given block at index/begin
    pub fn block_disk_locs(info: &Arc<Info>, index: u32, begin: u32) -> LocIter {
        let len = info.block_len(index, begin);
        LocIter::new(info.clone(), index, begin, len)
    }

    /// Calculates the file offsets for a given piece at index
    pub fn piece_disk_locs(info: &Arc<Info>, index: u32) -> LocIter {
        let len = info.piece_len(index);
        LocIter::new(info.clone(), index, 0, len)
    }
}

pub struct LocIter {
    info: Arc<Info>,
    state: LocIterState,
}

enum LocIterState {
    P(LocIterPos),
    Done,
}

struct LocIterPos {
    len: u64,
    data_start: u64,
    fidx: u64,
    file: usize,
}

impl LocIter {
    pub fn new(info: Arc<Info>, index: u32, begin: u32, len: u32) -> LocIter {
        let len = u64::from(len);
        // The current file end length.
        let (mut file, mut fidx) = info.piece_idx[index as usize];
        fidx += begin as u64;
        while info.files[file].length < fidx {
            fidx -= info.files[file].length;
            file += 1;
        }

        let p = LocIterPos {
            len,
            data_start: 0,
            fidx,
            file,
        };

        LocIter {
            info,
            state: LocIterState::P(p),
        }
    }
}

impl Iterator for LocIter {
    type Item = disk::Location;

    fn next(&mut self) -> Option<Self::Item> {
        match mem::replace(&mut self.state, LocIterState::Done) {
            LocIterState::P(mut p) => {
                let f_len = self.info.files[p.file].length;
                let file_write_len = cmp::min(f_len - p.fidx, p.len);

                if file_write_len == p.len {
                    // The file is longer than our len, just write to it,
                    // exit loop
                    Some(disk::Location::new(
                        p.file,
                        self.info.files[p.file].length,
                        p.fidx,
                        p.data_start,
                        p.data_start + file_write_len,
                        self.info.clone(),
                    ))
                } else {
                    // Write to the end of file, continue
                    let res = disk::Location::new(
                        p.file,
                        self.info.files[p.file].length,
                        p.fidx,
                        p.data_start,
                        p.data_start + file_write_len,
                        self.info.clone(),
                    );

                    // Use the next file, updating state as needed
                    p.fidx -= self.info.files[p.file].length - file_write_len;
                    p.file += 1;
                    p.len -= file_write_len;
                    p.data_start += file_write_len;

                    self.state = LocIterState::P(p);
                    Some(res)
                }
            }
            LocIterState::Done => None,
        }
    }
}

fn parse_bencode_files(mut data: BTreeMap<String, BEncode>) -> Result<Vec<File>, &'static str> {
    match data.remove("files").and_then(|l| l.into_list()) {
        Some(fs) => {
            let mut path = PathBuf::new();
            path.push(data.remove("name")
                .and_then(|v| v.into_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correct_piece_len() {
        let scale = 3;
        let pieces = 15;
        let mut info = Info::with_pieces_scale(pieces, scale);
        let end = 16_700u32;
        info.total_len += end as u64;
        info.hashes.push(vec![]);
        for i in 0..pieces {
            assert_eq!(info.piece_len(i), info.piece_len);
            for o in 0..scale {
                assert_eq!(info.block_len(i, o * 16_384), 16_384);
            }
        }
        assert_eq!(info.piece_len(pieces), end as u32);
        assert_eq!(info.block_len(pieces, 0), 16_384);
        assert_eq!(info.block_len(pieces, 16_384), (end % 16_384) as u32);
    }

    #[test]
    fn loc_iter_bounds() {
        let mut info = Info::with_pieces(4);
        info.files.clear();
        info.files.push(File {
            path: PathBuf::from(""),
            length: 40000,
        });
        info.files.push(File {
            path: PathBuf::from(""),
            length: 10000,
        });
        info.total_len = 50000;
        info.piece_idx =
            Info::generate_piece_idx(info.hashes.len(), info.piece_len as u64, &info.files);
        let info = Arc::new(info);
        let mut locs = Info::block_disk_locs(&info, 0, 0);
        let n = locs.next().unwrap();
        assert_eq!(n.start, 0);
        assert_eq!(n.end, 16384);
        assert_eq!(n.file, 0);
        assert_eq!(n.offset, 0);
        assert_eq!(locs.next().is_none(), true);

        let mut locs = Info::block_disk_locs(&info, 1, 0);
        let n = locs.next().unwrap();
        assert_eq!(n.start, 0);
        assert_eq!(n.end, 16384);
        assert_eq!(n.file, 0);
        assert_eq!(n.offset, 16384);
        assert_eq!(locs.next().is_none(), true);

        let mut locs = Info::block_disk_locs(&info, 2, 0);
        let n = locs.next().unwrap();
        assert_eq!(n.start, 0);
        assert_eq!(n.end, 7232);
        assert_eq!(n.file, 0);
        assert_eq!(n.offset, 16384 * 2);

        let n = locs.next().unwrap();
        assert_eq!(n.start, 7232);
        assert_eq!(n.end, 16384);
        assert_eq!(n.file, 1);
        assert_eq!(n.offset, 0);

        let mut locs = Info::block_disk_locs(&info, 3, 0);
        let n = locs.next().unwrap();
        assert_eq!(n.start, 0);
        assert_eq!(n.end, 848);
        assert_eq!(n.file, 1);
        assert_eq!(n.offset, 16384 - 7232);
    }
}
