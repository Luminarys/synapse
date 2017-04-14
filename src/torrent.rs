use bencode::BEncode;
use std::path::PathBuf;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct File {
    path: PathBuf,
    length: usize,
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
            }
            return Ok(files);
        }
        None => File::from_bencode(BEncode::Dict(data)).map(|f| vec![f]),
    }
}

#[derive(Clone, Debug)]
pub struct Torrent {
    announce: String,
    piece_len: usize,
    hashes: Vec<Vec<u8>>,
    files: Vec<File>,
}

impl Torrent {
    pub fn from_bencode(data: BEncode) -> Result<Torrent, &'static str> {
        data.to_dict()
            .and_then(|mut d| d.remove("info").and_then(|i| i.to_dict()).map(|i| (d, i)))
            .ok_or("")
            .and_then(|(mut d, mut i)| {
                let a = d.remove("announce")
                    .and_then(|a| a.to_string())
                    .ok_or("Torrent must have announce url")?;
                let pl = i.remove("piece length")
                    .and_then(|i| i.to_int())
                    .ok_or("Torrent must specify piece length")?;
                let hashes = i.remove("pieces")
                    .and_then(|p| p.to_bytes())
                    .map(|mut p| {
                        println!("Hashes len: {:?}", p.len());
                        let mut v = Vec::new();
                        while p.len() != 0 {
                            let remaining = p.split_off(20);
                            v.push(p);
                            p = remaining;
                        };
                        v
                    })
                    .ok_or("Torrent must provide valid hashes")?;
                let files = parse_bencode_files(i)?;
                Ok(Torrent {
                    announce: a,
                    piece_len: pl as usize,
                    hashes: hashes,
                    files: files,
                })
            })
    }
}
