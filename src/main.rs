extern crate axon;
extern crate mio;
extern crate slab;

mod bencode;
mod torrent;
mod ev_loop;

use std::env;
use std::fs::File;
use std::io;
use torrent::Torrent;
use ev_loop::EvLoop;

fn main() {
    let torrent = env::args().nth(1).unwrap();
    download_torrent(&torrent);
}

fn download_torrent(path: &str) -> Result<(), io::Error> {
    let mut data = File::open(path)?;
    let t = Torrent::from_bencode(bencode::decode(&mut data).unwrap()).unwrap();
    let mut e = EvLoop::new()?;
    e.add_torrent(t);
    e.run()?;
    Ok(())
}
