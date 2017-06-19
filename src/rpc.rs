use std::sync::mpsc;
use std::{thread, time};
use bencode::{self, BEncode};
use slog::Logger;
use {amy, tiny_http, serde_json, control, CONTROL, torrent, CONFIG, TC};

pub struct Handle {
    pub tx: mpsc::Sender<Response>,
    pub rtx: mpsc::Sender<Request>,
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
    rrx: mpsc::Receiver<Request>,
    tx: amy::Sender<control::Request>,
    l: Logger,
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
    pub fn new(rx: mpsc::Receiver<Response>, rrx: mpsc::Receiver<Request>, l: Logger) -> RPC {
        RPC {
            rx,
            rrx,
            tx: CONTROL.ctrl_tx.lock().unwrap().try_clone().unwrap(),
            l,
        }
    }

    pub fn run(&mut self) {
        debug!(self.l, "Awaiting requests");
        let server = tiny_http::Server::http(("0.0.0.0", CONFIG.get().rpc_port)).unwrap();
        while let Ok(pr) = server.recv_timeout(time::Duration::from_secs(1)) {
            if let Some(r) = pr {
                self.handle_request(r);
            } else {
                match self.rrx.try_recv() {
                    Ok(Request::Shutdown) => {
                        debug!(self.l, "Shutting down!");
                        return;
                    }
                    _ => { }
                }
            }
        }
    }

    fn handle_request(&mut self, mut request: tiny_http::Request) {
        debug!(self.l, "New Req {:?}, {:?}!", request.url(), request.method());
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
        if request.url() == "/torrent" {
            let mut data = Vec::new();
            request.as_reader().read_to_end(&mut data).unwrap();
            resp = match bencode::decode_buf(&mut data) {
                Ok(b) => Ok(Request::AddTorrent(b)),
                Err(_) => Err("Bad torrent!".to_owned()),
            };
        }

        let resp = match resp {
            Ok(rpc) => {
                if let Ok(()) = self.tx.send(control::Request::RPC(rpc)) {
                    let resp = self.rx.recv().unwrap();
                    serde_json::to_string(&resp).unwrap()
                } else {
                    serde_json::to_string(&Response::Err("Shutting down!".to_owned())).unwrap()
                }
            }
            Err(e) => serde_json::to_string(&Response::Err(e)).unwrap(),
        };
        debug!(self.l, "Request handled!");
        let mut resp = tiny_http::Response::from_string(resp);
        let cors_o = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Origin"[..], &b"*"[..]).unwrap();
        let cors_m = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Methods"[..], &b"POST, GET"[..]).unwrap();
        let cors_h = tiny_http::Header::from_bytes(&b"Access-Control-Allow-Headers"[..], &b"Content-Type"[..]).unwrap();
        resp.add_header(cors_o);
        resp.add_header(cors_m);
        resp.add_header(cors_h);
        request.respond(resp).unwrap();
    }
}

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

pub fn start(l: Logger) -> Handle {
    debug!(l, "Initializing!");
    let (tx, rx) = mpsc::channel();
    let (rtx, rrx) = mpsc::channel();
    thread::spawn(move || {
        let mut d = RPC::new(rx, rrx, l);
        d.run();
        use std::sync::atomic;
        TC.fetch_sub(1, atomic::Ordering::SeqCst);
    });
    Handle { tx, rtx }
}
