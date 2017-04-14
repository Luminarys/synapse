extern crate axon;
extern crate num_cpus;

mod bencode;
mod torrent;

use std::env;
use std::fs::File;
use std::io;
use torrent::Torrent;

fn main() {
    let torrent = env::args().nth(1).unwrap();
    download_torrent(&torrent);
}

fn download_torrent(path: &str) -> Result<(), io::Error> {
    let mut data = File::open(path)?;
    let f = Torrent::from_bencode(bencode::decode(&mut data).unwrap()).unwrap();
    println!("Files: {:?}", f.files);
    Ok(())
}
