use std::sync::mpsc;
use std::thread;
use std::io::ErrorKind;
use std::net::{SocketAddrV4, Ipv4Addr, TcpListener};
use std::sync::atomic;
use amy::{self, Poller, Registrar};
use std::collections::HashMap;
use bencode::BEncode;

pub struct Handle {
    pub tx: mpsc::Sender<Response>,
}

impl Handle {
    pub fn init(&self) { }

    pub fn get(&self) -> mpsc::Sender<Response> {
        self.tx.clone()
    }
}

unsafe impl Sync for Handle {}

pub struct RPC {
    rx: mpsc::Receiver<Response>,
}

impl RPC {
    pub fn new(rx: mpsc::Receiver<Response>) -> RPC {
        RPC {
            rx
        }
    }

    pub fn run(&mut self) {
    
    }
}

pub enum Request {
    ListTorrents,
    TorrentInfo(usize),
    AddTorrent(BEncode),
    StopTorrent(usize),
    RemoveTorrent(usize),
    ThrottleUpload(usize),
    ThrottleDownload(usize),
}

#[derive(Serialize, Debug)]
pub enum Response {
    Torrents(Vec<usize>),
    TorrentInfo(TorrentInfo),
    AddResult(Result<usize, &'static str>),
    Ack,
    Err(&'static str),
}

#[derive(Serialize, Debug)]
pub struct TorrentInfo {
    pub name: String,
    pub status: Status,
    pub size: usize,
    pub downloaded: usize,
    pub uploaded: usize,
    pub tracker: String,
}

#[derive(Serialize, Debug)]
pub enum Status {
    Downloading,
    Seeding,
    Paused,
}

pub fn start() -> Handle {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut d = RPC::new(rx);
        d.run();
    });
    Handle { tx }
}
