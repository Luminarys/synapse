use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::{cmp, fmt, mem};

use rand::{self, Rng};
use url::Url;

use crate::bencode::BEncode;
use crate::disk;
use crate::util::{hash_to_id, id_to_hash, sha1_hash};

#[derive(Clone)]
pub struct Info {
    pub name: String,
    pub announce: Option<Arc<Url>>,
    pub creator: Option<String>,
    pub comment: Option<String>,
    pub piece_len: u32,
    pub total_len: u64,
    pub hashes: Vec<Vec<u8>>,
    pub hash: [u8; 20],
    pub files: Vec<File>,
    pub private: bool,
    pub be_name: Option<Vec<u8>>,
    /// Maps piece idx -> file idx + file offset
    pub piece_idx: Vec<(usize, u64)>,
    pub url_list: Vec<Vec<Arc<Url>>>,
}

impl fmt::Debug for Info {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        match (
            d.remove(b"name".as_ref()),
            d.remove(b"path".as_ref()),
            d.remove(b"length".as_ref()),
        ) {
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
                        dir.into_string()
                            .ok_or("File path parts should be strings")?,
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
            return Err("magnet URL must use magnet URL scheme");
        };
        let hash =
            url.query_pairs()
                .find(|&(ref k, ref v)| k == "xt" && v.starts_with("urn:btih:"))
                .and_then(|(_, ref v)| {
                    id_to_hash(&v[9..]).or_else(|| {
                        base32::decode(base32::Alphabet::RFC4648 { padding: true }, &v[9..])
                            .and_then(|b| {
                                if b.len() != 20 {
                                    return None;
                                }
                                let mut a = [0; 20];
                                (&mut a[..]).copy_from_slice(&b);
                                Some(a)
                            })
                    })
                })
                .ok_or("No hash found in magnet")?;

        let mut url_list: Vec<_> = url
            .query_pairs()
            .filter(|&(ref k, _)| k == "tr")
            .filter_map(|(_, ref v)| Url::parse(v).ok())
            .map(Arc::new)
            .collect();
        rand::thread_rng().shuffle(&mut url_list[..]);

        let name = url
            .query_pairs()
            .find(|&(ref k, _)| k == "dn")
            .map(|(_, ref v)| v.to_string())
            .unwrap_or_else(|| "".to_owned());
        Ok(Info {
            name,
            comment: None,
            creator: None,
            announce: None,
            piece_len: 0,
            total_len: 0,
            hashes: vec![],
            hash,
            files: vec![],
            private: false,
            be_name: None,
            piece_idx: vec![],
            url_list: vec![url_list],
        })
    }

    pub fn complete(&self) -> bool {
        !self.hashes.is_empty()
    }

    pub fn to_torrent_bencode(&self) -> BEncode {
        let mut torrent = BTreeMap::new();
        let info = self.to_bencode();
        self.announce.as_ref().map(|url| {
            torrent.insert(
                b"announce".to_vec(),
                BEncode::String(url.as_str().as_bytes().to_owned()),
            )
        });
        torrent.insert(b"info".to_vec(), info);
        BEncode::Dict(torrent)
    }

    pub fn to_bencode(&self) -> BEncode {
        let mut info = BTreeMap::new();
        if let Some(ref n) = self.be_name {
            info.insert(b"name".to_vec(), BEncode::String(n.clone()));
        }
        if self.private {
            info.insert(b"private".to_vec(), BEncode::Int(1));
        }
        info.insert(
            b"piece length".to_vec(),
            BEncode::Int(i64::from(self.piece_len)),
        );
        let mut pieces = Vec::with_capacity(self.hashes.len() * 20);
        for h in &self.hashes {
            pieces.extend_from_slice(h);
        }
        info.insert(b"pieces".to_vec(), BEncode::String(pieces));
        if self.files.len() == 1 {
            info.insert(
                b"length".to_vec(),
                BEncode::Int(self.files[0].length as i64),
            );
        } else {
            let files = self
                .files
                .iter()
                .map(|f| {
                    let mut fb = BTreeMap::new();
                    fb.insert(b"length".to_vec(), BEncode::Int(f.length as i64));
                    fb.insert(
                        b"path".to_vec(),
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
            info.insert(b"files".to_vec(), BEncode::List(files));
        }
        BEncode::Dict(info)
    }

    pub fn from_bencode(data: BEncode) -> Result<Info, &'static str> {
        data.into_dict()
            .and_then(|mut d| {
                d.remove(b"info".as_ref())
                    .and_then(|i| i.into_dict())
                    .map(|i| (d, i))
            })
            .ok_or("invalid info field")
            .and_then(|(mut d, mut i)| {
                let mut info_bytes = Vec::new();
                BEncode::Dict(i.clone()).encode(&mut info_bytes).unwrap();
                let hash = sha1_hash(&info_bytes);

                let announce = d
                    .remove(b"announce".as_ref())
                    .and_then(BEncode::into_string)
                    .and_then(|a| Url::parse(&a).ok().map(Arc::new));
                let comment = d.remove(b"comment".as_ref()).and_then(|b| b.into_string());
                let creator = d
                    .remove(b"created by".as_ref())
                    .and_then(|b| b.into_string());
                let pl = i
                    .remove(b"piece length".as_ref())
                    .and_then(|i| i.into_int())
                    .ok_or("Info must specify piece length")? as u64;
                let hashes = i
                    .remove(b"pieces".as_ref())
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
                        if !s.is_empty() {
                            return None;
                        }
                        Some(v)
                    })
                    .ok_or("Info must provide valid hashes")?;

                let private = if let Some(v) = i.remove(b"private".as_ref()) {
                    v.into_int()
                        .and_then(|p| {
                            if p == 0 {
                                Some(false)
                            } else if p == 1 {
                                Some(true)
                            } else {
                                None
                            }
                        })
                        .ok_or("private key must be an integer equal to 0 or 1 if present!")?
                } else {
                    false
                };

                let be_name = if let Some(v) = i.get(b"name".as_ref()).cloned() {
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

                let url_list: Vec<_> = d
                    .remove(b"announce-list".as_ref())
                    .and_then(BEncode::into_list)
                    .unwrap_or_else(Vec::new)
                    .into_iter()
                    .map(|l| {
                        let mut l: Vec<_> = l
                            .into_list()
                            .unwrap_or_else(Vec::new)
                            .into_iter()
                            .filter_map(BEncode::into_string)
                            .filter_map(|s| Url::parse(&s).ok().map(Arc::new))
                            .collect();
                        rand::thread_rng().shuffle(&mut l[..]);
                        l
                    })
                    .collect();

                Ok(Info {
                    name,
                    comment,
                    creator,
                    announce,
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
            comment: None,
            creator: None,
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
            comment: None,
            creator: None,
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
        LocIter::new(info.clone(), None, index, begin, len)
    }

    pub fn block_disk_locs_pri(
        info: &Arc<Info>,
        priorities: &Arc<Vec<u8>>,
        index: u32,
        begin: u32,
    ) -> LocIter {
        let len = info.block_len(index, begin);
        LocIter::new(info.clone(), Some(priorities.clone()), index, begin, len)
    }

    /// Calculates the file offsets for a given piece at index
    pub fn piece_disk_locs(info: &Arc<Info>, index: u32) -> LocIter {
        let len = info.piece_len(index);
        LocIter::new(info.clone(), None, index, 0, len)
    }
}

pub struct LocIter {
    info: Arc<Info>,
    priorities: Option<Arc<Vec<u8>>>,
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
    pub fn new(
        info: Arc<Info>,
        priorities: Option<Arc<Vec<u8>>>,
        index: u32,
        begin: u32,
        len: u32,
    ) -> LocIter {
        let len = u64::from(len);
        // The current file end length.
        let (mut file, mut fidx) = info.piece_idx[index as usize];
        fidx += u64::from(begin);
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
            priorities,
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
                        self.priorities
                            .as_ref()
                            .map(|pri| pri[p.file] != 0)
                            .unwrap_or(false),
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
                        self.priorities
                            .as_ref()
                            .map(|pri| pri[p.file] != 0)
                            .unwrap_or(false),
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

fn parse_bencode_files(mut data: BTreeMap<Vec<u8>, BEncode>) -> Result<Vec<File>, &'static str> {
    match data.remove(b"files".as_ref()).and_then(|l| l.into_list()) {
        Some(fs) => {
            let mut path = PathBuf::new();
            path.push(
                data.remove(b"name".as_ref())
                    .and_then(|v| v.into_string())
                    .ok_or("Multifile mode must have a name field")?,
            );
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
