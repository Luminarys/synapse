use std::sync::mpsc;
use std::thread;
use bencode::BEncode;
use {amy, tiny_http, serde_json, control, CONTROL};

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
    tx: amy::Sender<control::Request>,
}

impl RPC {
    pub fn new(rx: mpsc::Receiver<Response>) -> RPC {
        RPC {
            rx,
            tx: CONTROL.ctrl_tx(),
        }
    }

    pub fn run(&mut self) {
        let server = tiny_http::Server::http("0.0.0.0:5432").unwrap();

        for request in server.incoming_requests() {
            if request.url() == "/torrent/list" {
                self.tx.send(control::Request::RPC(Request::ListTorrents));
            } else if request.url().starts_with("/torrent/info/") {
            } else if request.url().starts_with("/torrent/stop/") {
            } else if request.url().starts_with("/torrent/remove/") {
            } else if request.url().starts_with("/torrent") && request.method() == &tiny_http::Method::Post {
            } else {
                let response = tiny_http::Response::from_string("Go away!");
                request.respond(response);
                continue;
            }
            let resp = self.rx.recv().unwrap();
            let s = serde_json::to_string(&resp).unwrap();
            let response = tiny_http::Response::from_string(s);
            request.respond(response);
        }
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
