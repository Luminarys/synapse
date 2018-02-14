mod http;
mod udp;
mod errors;
mod dns;
mod dht;

use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::{io, result, thread};

use byteorder::{BigEndian, ReadBytesExt};
use url::Url;
use amy;

pub use self::errors::{Error, ErrorKind, Result, ResultExt};
use torrent::Torrent;
use bencode::BEncode;
use control::cio;
use util::AView;
use handle;
use disk;
use CONFIG;

pub struct Tracker {
    poll: amy::Poller,
    ch: handle::Handle<Request, Response>,
    dns_res: amy::Receiver<dns::QueryResponse>,
    http: http::Handler,
    queue: VecDeque<Announce>,
    udp: udp::Handler,
    dht: dht::Manager,
    dns: dns::Resolver,
    timer: usize,
    shutting_down: bool,
}

#[derive(Debug)]
pub enum Request {
    Announce(Announce),
    GetPeers(GetPeers),
    AddNode(SocketAddr),
    DHTAnnounce([u8; 20]),
    Shutdown,
}

#[derive(Debug)]
pub struct Announce {
    id: usize,
    url: AView<Url>,
    hash: [u8; 20],
    port: u16,
    uploaded: u64,
    downloaded: u64,
    left: u64,
    num_want: Option<u16>,
    event: Option<Event>,
}

#[derive(Debug)]
pub struct GetPeers {
    pub id: usize,
    pub hash: [u8; 20],
}

#[derive(Debug)]
pub enum Event {
    Started,
    Stopped,
    Completed,
}

#[derive(Debug)]
pub enum Response {
    Tracker {
        tid: usize,
        url: AView<Url>,
        resp: Result<TrackerResponse>,
    },
    DHT {
        tid: usize,
        peers: Vec<SocketAddr>,
    },
}

#[derive(Debug)]
pub struct TrackerResponse {
    pub peers: Vec<SocketAddr>,
    pub interval: u32,
    pub leechers: u32,
    pub seeders: u32,
}

const POLL_INT_MS: usize = 1000;

impl Tracker {
    pub fn start(
        creg: &mut amy::Registrar,
        db: amy::Sender<disk::Request>,
    ) -> io::Result<(handle::Handle<Response, Request>, thread::JoinHandle<()>)> {
        let poll = amy::Poller::new()?;
        let mut reg = poll.get_registrar()?;
        let (ch, dh) = handle::Handle::new(creg, &mut reg)?;
        let timer = reg.set_interval(150)?;
        let (dtx, drx) = reg.channel()?;
        let udp = udp::Handler::new(&reg)?;
        let dht = dht::Manager::new(&reg, db)?;
        let http = http::Handler::new(&reg)?;
        let dns = dns::Resolver::new(dtx);
        let th = dh.run("trk", move |h| {
            Tracker {
                poll,
                ch: h,
                udp,
                dht,
                http,
                dns,
                dns_res: drx,
                timer,
                queue: VecDeque::new(),
                shutting_down: false,
            }.run()
        })?;
        Ok((ch, th))
    }

    pub fn run(&mut self) {
        self.dht.init();

        debug!("Initialized!");
        'outer: loop {
            match self.poll.wait(POLL_INT_MS) {
                Ok(events) => for event in events {
                    if self.handle_event(event).is_err() {
                        break 'outer;
                    }
                },
                Err(e) => {
                    error!("Failed to poll for events: {:?}", e);
                }
            }
        }

        debug!("Shutting down!");
        self.shutting_down = true;

        // Shutdown loop - wait for all requests to complete
        loop {
            for event in self.poll.wait(POLL_INT_MS).unwrap() {
                if self.handle_event(event).is_err() {}
                if self.http.complete() && self.udp.complete() {
                    return;
                }
            }
        }
    }

    fn handle_event(&mut self, event: amy::Notification) -> result::Result<(), ()> {
        if event.id == self.ch.rx.get_id() {
            return self.handle_request();
        } else if event.id == self.dns_res.get_id() {
            self.handle_dns_res();
        } else if event.id == self.timer {
            self.handle_timer();
        } else {
            self.handle_socket(event);
        }
        Ok(())
    }

    fn handle_request(&mut self) -> result::Result<(), ()> {
        while let Ok(r) = self.ch.recv() {
            match r {
                Request::Announce(req) => self.handle_announce(req),
                Request::GetPeers(gp) => {
                    debug!("Handling dht peer find req!");
                    self.dht.get_peers(gp.id, gp.hash);
                }
                Request::AddNode(addr) => {
                    debug!("Handling dht node addition req!");
                    self.dht.add_addr(addr);
                }
                Request::DHTAnnounce(hash) => {
                    debug!("Handling dht announce req!");
                    self.dht.announce(hash);
                }
                Request::Shutdown => {
                    return Err(());
                }
            }
        }
        Ok(())
    }

    fn handle_announce(&mut self, req: Announce) {
        debug!("Handling announce request!");
        if self.udp.active_requests() + self.http.active_requests() > CONFIG.net.max_open_announces
        {
            self.queue.push_back(req);
        } else {
            let id = req.id;
            let url = req.url.clone();
            let response = match url.scheme() {
                "http" | "https" => self.http.new_announce(req, &mut self.dns),
                "udp" => self.udp.new_announce(req, &mut self.dns),
                s => Err(
                    ErrorKind::InvalidRequest(format!("Unknown tracker url scheme: {}", s)).into(),
                ),
            };
            if let Err(e) = response {
                self.send_response(Response::Tracker {
                    tid: id,
                    url,
                    resp: Err(e),
                });
            }
        }
    }

    fn dequeue_req(&mut self) {
        // Attempt to dequeue next request if we can
        if let Some(a) = self.queue.pop_front() {
            self.handle_announce(a);
        }
    }

    fn handle_dns_res(&mut self) {
        while let Ok(r) = self.dns_res.try_recv() {
            let resp = if self.http.contains(r.id) {
                self.http.dns_resolved(r)
            } else if self.udp.contains(r.id) {
                self.udp.dns_resolved(r)
            } else {
                None
            };
            if let Some(r) = resp {
                self.send_response(r);
            }
        }
    }

    fn handle_timer(&mut self) {
        for r in self.http
            .tick()
            .into_iter()
            .chain(self.udp.tick().into_iter())
        {
            self.send_response(r);
        }

        self.dns.tick();
        self.dht.tick();
    }

    fn handle_socket(&mut self, event: amy::Notification) {
        if self.http.contains(event.id) {
            let resp = if event.event.readable() {
                self.http.readable(event.id, &mut self.dns)
            } else {
                self.http.writable(event.id)
            };
            if let Some(r) = resp {
                self.send_response(r);
            }
        } else if self.udp.id() == event.id {
            for resp in self.udp.readable() {
                self.send_response(resp);
            }
        } else if self.dht.id() == event.id {
            for resp in self.dht.readable() {
                self.send_response(resp);
            }
        } else {
            unreachable!();
        };
    }

    fn send_response(&mut self, r: Response) {
        if !self.shutting_down {
            debug!("Sending trk response to control!");
            self.ch.send(r).ok();
        }
        // TODO: The active announce queue could grow with DHT usage,
        // since DHT stuff doesn't go into the announce queue, but still triggers send_response.
        // Not a big deal, but worth thinking about for later.
        self.dequeue_req();
    }
}

impl Request {
    pub fn new_announce<T: cio::CIO>(
        torrent: &Torrent<T>,
        event: Option<Event>,
    ) -> Option<Request> {
        let url = if let Some(trk) = torrent.trackers().front() {
            trk.url.clone()
        } else {
            return None;
        };
        Some(Request::Announce(Announce {
            id: torrent.id(),
            url,
            hash: torrent.info().hash,
            port: CONFIG.port,
            uploaded: torrent.uploaded(),
            downloaded: torrent.downloaded(),
            // This should be fine because the true len is usually slightly less than
            // piece_len * pieces_dld (due to shorter last piece), so we always get
            // either the correct amount left or 0.
            left: torrent.info().total_len.saturating_sub(
                torrent.pieces().iter().count() as u64 * torrent.info().piece_len as u64,
            ),
            // TODO: Develop better heuristics here.
            // For now, only request peers if we're leeching,
            // let existing peers connect otherwise
            num_want: if torrent.complete() { None } else { Some(50) },
            event,
        }))
    }

    pub fn started<T: cio::CIO>(torrent: &Torrent<T>) -> Option<Request> {
        Request::new_announce(torrent, Some(Event::Started))
    }

    pub fn stopped<T: cio::CIO>(torrent: &Torrent<T>) -> Option<Request> {
        Request::new_announce(torrent, Some(Event::Stopped))
    }

    pub fn completed<T: cio::CIO>(torrent: &Torrent<T>) -> Option<Request> {
        Request::new_announce(torrent, Some(Event::Completed))
    }

    pub fn interval<T: cio::CIO>(torrent: &Torrent<T>) -> Option<Request> {
        Request::new_announce(torrent, None)
    }

    pub fn custom<T: cio::CIO>(torrent: &Torrent<T>, url: AView<Url>) -> Option<Request> {
        Request::new_announce(torrent, None).map(|mut r| {
            match r {
                Request::Announce(ref mut a) => a.url = url,
                _ => {}
            }
            r
        })
    }
}

impl TrackerResponse {
    pub fn empty() -> TrackerResponse {
        TrackerResponse {
            peers: vec![],
            interval: 900,
            leechers: 0,
            seeders: 0,
        }
    }

    pub fn from_bencode(data: BEncode) -> Result<TrackerResponse> {
        let mut d = data.into_dict().ok_or(ErrorKind::InvalidResponse(
            "Tracker response must be a dictionary type!",
        ))?;
        if let Some(BEncode::String(data)) = d.remove("failure reason") {
            let reason = String::from_utf8(data)
                .chain_err(|| ErrorKind::InvalidResponse("Failure reason must be UTF8!"))?;
            return Err(ErrorKind::TrackerError(reason).into());
        }
        let mut resp = TrackerResponse::empty();
        match d.remove("peers") {
            Some(BEncode::String(ref data)) => for p in data.chunks(6) {
                let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
                let socket = SocketAddrV4::new(ip, (&p[4..]).read_u16::<BigEndian>().unwrap());
                resp.peers.push(SocketAddr::V4(socket));
            },
            _ => {}
        };
        match d.remove("interval") {
            Some(BEncode::Int(ref i)) => {
                resp.interval = *i as u32;
            }
            _ => {
                return Err(ErrorKind::InvalidResponse("Response must have interval!").into());
            }
        };
        Ok(resp)
    }
}
