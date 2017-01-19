#[macro_use]
extern crate lazy_static;
extern crate byteorder;
extern crate vecio;
extern crate rand;
extern crate sha1;
extern crate mio;
extern crate slab;

mod peer;
mod torrent;
mod piece_field;
mod manager;
mod message;
mod worker;
mod disk;
mod pool;
mod config;
mod handle;
mod util;

pub use handle::Handle;

lazy_static! {
    pub static ref PEER_ID: [u8; 20] = {
        use rand::{self, Rng};

        let mut pid = [0u8; 20];
        let prefix = b"-AN0001-";
        for i in 0..prefix.len() {
            pid[i] = prefix[i];
        }

        let mut rng = rand::thread_rng();
        for i in 8..19 {
            pid[i] = rng.gen::<u8>();
        }
        pid
    };

    pub static ref MEM_POOL: pool::Pool = pool::Pool::new();
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
    }
}
