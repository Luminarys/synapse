#[macro_use]
extern crate axon;
extern crate num_cpus;

mod bencode;

use std::env;
use std::fs::File;
use std::io;
use std::io::Read;

fn main() {
    let torrent = env::args().nth(1).unwrap();
}

fn download_torrent(path: &str) -> Result<(), io::Error> {
    let data = File::open(path)?.bytes();
    Ok(())
}
