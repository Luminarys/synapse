pub mod ws;
pub mod resource;
pub mod criterion;
pub mod message;
pub mod error;

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

