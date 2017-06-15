use std::sync::mpsc;
use std::thread;
use bencode::{self, BEncode};
use {amy, tiny_http, serde_json, control, CONTROL, torrent};

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

macro_rules! id_match {
    ($req:expr, $resp:expr, $s:expr, $body:expr) => (
        {
            lazy_static! {
                static ref M: (String, String, usize) = {
                    let mut s = $s.to_owned();
                    let idx = s.find("{}").unwrap();
                    let mut remaining = s.split_off(idx);
                    let end = remaining.split_off(2);
                    (s, end, idx)
                };
            };
            let ref start = M.0;
            let ref end = M.1;
            let idx = M.2;
            if $req.url().starts_with(start) && $req.url().ends_with(end) {
                let len = $req.url().len();
                let val = &$req.url()[idx..(len - end.len())];
                if let Ok(i) = val.parse::<usize>() {
                    $resp = Ok($body(i));
                } else {
                    $resp = Err(format!("{} is not a valid integer!", val));
                }
            }
        }
    );
}

impl RPC {
    pub fn new(rx: mpsc::Receiver<Response>) -> RPC {
        RPC {
            rx,
            tx: CONTROL.ctrl_tx.lock().unwrap().try_clone().unwrap(),
        }
    }

    pub fn run(&mut self) {
        let server = tiny_http::Server::http("0.0.0.0:8412").unwrap();
        for mut request in server.incoming_requests() {
            let mut resp = Err("Invalid URL".to_owned());
            id_match!(request, resp, "/torrent/{}/info", |i| Request::TorrentInfo(i));
            id_match!(request, resp, "/torrent/{}/pause", |i| Request::PauseTorrent(i));
            id_match!(request, resp, "/torrent/{}/resume", |i| Request::ResumeTorrent(i));
            id_match!(request, resp, "/torrent/{}/remove", |i| Request::RemoveTorrent(i));
            id_match!(request, resp, "/throttle/upload/{}", |i| Request::ThrottleUpload(i));
            id_match!(request, resp, "/throttle/download/{}", |i| Request::ThrottleDownload(i));
            if request.url() == "/torrent/list" {
                resp = Ok(Request::ListTorrents);
            };
            if request.url() == "/torrent" && request.method() == &tiny_http::Method::Post {
                let mut data = Vec::new();
                request.as_reader().read_to_end(&mut data).unwrap();
                resp = match bencode::decode_buf(&mut data) {
                    Ok(b) => Ok(Request::AddTorrent(b)),
                    Err(_) => Err("Bad torrent!".to_owned()),
                };
            }

            let resp = match resp {
                Ok(rpc) => {
                    self.tx.send(control::Request::RPC(rpc)).unwrap();
                    let resp = self.rx.recv().unwrap();
                    serde_json::to_string(&resp).unwrap()
                }
                Err(e) => serde_json::to_string(&Response::Err(e)).unwrap(),
            };
            let mut resp = tiny_http::Response::from_string(resp);
            let cors = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
            resp.add_header(cors);
            request.respond(resp).unwrap();
        }
    }
}

pub enum Request {
    ListTorrents,
    TorrentInfo(usize),
    AddTorrent(BEncode),
    PauseTorrent(usize),
    ResumeTorrent(usize),
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
    Err(String),
}

#[derive(Serialize, Debug)]
pub struct TorrentInfo {
    pub name: String,
    pub status: Status,
    pub size: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub tracker: String,
    pub tracker_status: torrent::TrackerStatus,
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
