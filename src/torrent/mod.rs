use bencode::BEncode;
use std::path::PathBuf;
use std::collections::BTreeMap;
use std::{io, fs};

mod parse;

#[derive(Clone, Debug)]
pub struct File {
    path: PathBuf,
    length: usize,
}

impl File {
    fn from_bencode(data: BEncode) -> Result<File, &'static str> {
        parse::file_from_bencode(data)
    }

    fn create(&self) -> Result<(), io::Error> {
        let f = fs::File::open(&self.path)?;
        f.set_len(self.length as u64)?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Torrent {
    pub announce: String,
    pub piece_len: usize,
    pub hashes: Vec<Vec<u8>>,
    pub files: Vec<File>,
}

impl Torrent {
    pub fn from_bencode(data: BEncode) -> Result<Torrent, &'static str> {
        parse::torrent_from_bencode(data)
    }

    pub fn create_files(&self) -> Result<(), io::Error> {
        for file in self.files.iter() {
            file.create()?;
        }
        Ok(())
    }
}
