extern crate axon;
extern crate mio;
extern crate slab;
extern crate byteorder;
extern crate rand;
extern crate sha1;
extern crate url;
extern crate reqwest;
extern crate iovec;
#[macro_use]
extern crate lazy_static;

mod bencode;
mod torrent;
mod ev_loop;
mod util;
mod socket;
mod disk;

use std::env;
use std::fs::File;
use std::io;
use torrent::Torrent;
use ev_loop::EvLoop;

lazy_static! {
    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        let prefix = b"-SYN001-";
        for i in 0..prefix.len() {
            pid[i] = prefix[i];
        }

        let mut rng = rand::thread_rng();
        for i in 8..19 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };

    pub static ref DISK: disk::Handle = {
        disk::start()
    };

}

fn main() {
    // TODO: http://geocar.sdf1.org/fast-servers.html maybe?
    // This design could actually be really good
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
