use std::sync::mpsc;
use std::thread;
use bencode::{self, BEncode};
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
            tx: CONTROL.ctrl_tx.lock().unwrap().try_clone().unwrap(),
        }
    }

    pub fn run(&mut self) {
        let server = tiny_http::Server::http("0.0.0.0:8412").unwrap();
        for mut request in server.incoming_requests() {
            let mut resp = None;
            // TODO: CLEAN ALL THIS SHIT UP
            if request.url() == "/torrent/list" {
                self.tx.send(control::Request::RPC(Request::ListTorrents)).unwrap();
            } else if request.url().starts_with("/torrent/info/") {
                let res = {
                    let (_, id) = request.url().split_at(14);
                    id.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::TorrentInfo(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad ID!"); }
                    _ => { }
                };
            } else if request.url().starts_with("/torrent/pause/") {
                let res = {
                    let (_, id) = request.url().split_at(15);
                    id.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::PauseTorrent(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad ID!"); }
                    _ => { }
                };
            } else if request.url().starts_with("/torrent/resume/") {
                let res = {
                    let (_, id) = request.url().split_at(16);
                    id.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::ResumeTorrent(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad ID!"); }
                    _ => { }
                };
            } else if request.url().starts_with("/torrent/remove/") {
                let res = {
                    let (_, id) = request.url().split_at(16);
                    id.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::RemoveTorrent(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad ID!"); }
                    _ => { }
                };
            } else if request.url().starts_with("/torrent") && request.method() == &tiny_http::Method::Post {
                println!("Uploading torrent!");
                let mut data = Vec::new();
                request.as_reader().read_to_end(&mut data).unwrap();
                match bencode::decode_buf(&mut data) {
                    Ok(b) => {
                        self.tx.send(control::Request::RPC(Request::AddTorrent(b))).unwrap();
                    }
                    Err(_) => {
                        resp = Some("Bad torrent!");
                    },
                }
            } else if request.url().starts_with("/rate/upload/") {
                let res = {
                    let (_, amnt) = request.url().split_at(13);
                    amnt.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::ThrottleUpload(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad amount!"); }
                    _ => { }
                };
            } else if request.url().starts_with("/rate/download/") {
                let res = {
                    let (_, amnt) = request.url().split_at(15);
                    amnt.parse::<usize>().map(|i| {
                        self.tx.send(control::Request::RPC(Request::ThrottleDownload(i))).unwrap();
                    })
                };
                match res {
                    Err(_) => { resp = Some("Bad amount!"); }
                    _ => { }
                };
            } else {
                resp = Some("Invalid URL!");
            }

            let mut r = if let Some(e) = resp {
                let s = serde_json::to_string(&Response::Err(e)).unwrap();
                tiny_http::Response::from_string(s)
            } else {
                let resp = self.rx.recv().unwrap();
                let s = serde_json::to_string(&resp).unwrap();
                tiny_http::Response::from_string(s)
            };
            let cors = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
            r.add_header(cors);
            request.respond(r).unwrap();
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
