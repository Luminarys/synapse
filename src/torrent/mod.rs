mod info;
mod peer;
mod announcer;
mod piece_field;

use bencode::BEncode;
use self::peer::Peer;
use self::announcer::Announcer;
use slab::Slab;

pub struct Torrent {
    pub info: info::Info,
    peers: Slab<Peer, usize>,
    announcer: Announcer,
}

impl Torrent {
    pub fn from_bencode(data: BEncode) -> Result<Torrent, &'static str> {
        let info = info::Info::from_bencode(data)?;
        let peers = Slab::with_capacity(32);
        let announcer = Announcer::new().unwrap();
        Ok(Torrent {
            info: info,
            peers: peers,
            announcer: announcer,
        })
    }
}
