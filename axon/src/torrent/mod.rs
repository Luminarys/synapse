use piece_field::PieceField;
use slab::Slab;
use peer::Peer;

mod picker;

pub struct TorrentInfo {
    pub hash: [u8; 20],
    pub announce: String,
    pub created: Option<usize>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub pieces: PieceInfo,
}

pub struct PieceInfo {
    pub length: usize,
    pub pieces: Vec<u8>,
    pub private: bool,
    pub files: Vec<FileInfo>,
}

pub struct FileInfo {
    length: usize,
    path: String,
}

pub struct Torrent {
    status: TorrentStatus,
    info: TorrentInfo,
    peers: Slab<usize>,
    picker: picker::Picker,
}

impl Torrent {
    pub fn new(info: TorrentInfo, max_peers: usize) -> Torrent {
        Torrent {
            status: TorrentStatus::new((info.pieces.pieces.len()/20) as usize),
            peers: Slab::with_capacity(max_peers),
            info: info,
            picker: picker::Picker::new(),
        }
    }

    pub fn status<'a>(&'a self) -> &'a TorrentStatus {
        &self.status
    }

    pub fn info<'a>(&'a self) -> &'a TorrentInfo {
        &self.info
    }

    pub fn picker<'a>(&'a mut self) -> &'a mut Picker {
        &mut self.picker
    }

    pub fn insert_peer(&mut self, peer_idx: usize) -> Result<usize, usize> {
        self.peers.insert(peer_idx)
    }

    pub fn remove_peer(&mut self, peer_idx: usize) {
        self.peers.remove(peer_idx);
    }
}

pub struct TorrentStatus {
    pub pieces: PieceField,
}

impl TorrentStatus {
    fn new(pieces: usize) -> TorrentStatus {
        TorrentStatus {
            pieces: PieceField::new(pieces as u32),
        }
    }
}
