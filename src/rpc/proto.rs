use bencode::{self, BEncode};
use torrent;

#[derive(Debug)]
pub enum Request {
    ListTorrents,
    TorrentInfo(usize),
    AddTorrent(BEncode),
    PauseTorrent(usize),
    ResumeTorrent(usize),
    RemoveTorrent(usize),
    ThrottleUpload(usize),
    ThrottleDownload(usize),
    Shutdown,
}

#[derive(Serialize, Debug)]
pub enum Response {
    Torrents(Vec<usize>),
    TorrentInfo(TorrentInfo),
    Ack,
    Err(String),
}

#[derive(Serialize, Debug)]
pub struct TorrentInfo {
    pub name: String,
    pub status: torrent::Status,
    pub size: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub tracker: String,
    pub tracker_status: torrent::TrackerStatus,
}


#[derive(Default)]
pub struct Message {
    pub header: u8,
    pub len: u64,
    pub mask: Option<[u8; 4]>,
    pub data: Vec<u8>,
}

pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    OtherControl(u8),
    Other(u8),
}

impl Message {
    pub fn new() -> Message {
        Message {
            data: vec![0u8; 1024],
            ..Default::default()
        }
    }

    pub fn allocate(&mut self) {
        let cl = self.data.len();
        let len = self.len as usize;
        if len > cl {
            self.data.reserve_exact(len - cl);
            unsafe { self.data.set_len(len); }
        }
    }

    pub fn fin(&self) -> bool {
        self.header & 0x80 != 0
    }

    pub fn extensions(&self) -> bool {
        self.header & 0x70 == 0
    }

    pub fn opcode(&self) -> Opcode {
        match self.header & 0x0F {
            0 => Opcode::Continuation,
            1 => Opcode::Text,
            2 => Opcode::Binary,
            o @ 3...7 => Opcode::Other(o),
            8 => Opcode::Close,
            9 => Opcode::Ping,
            10 => Opcode::Pong,
            o => Opcode::OtherControl(o),
        }
    }

    pub fn masked(&self) -> bool {
        self.mask.is_some()
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn mask(&self) -> Option<[u8; 4]> {
        self.mask
    }
}
