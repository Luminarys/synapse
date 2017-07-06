mod reader;
mod writer;

use tracker::{self, Announce, Response, Result, ResultExt, Error, ErrorKind, dns};
use std::time::Duration;
use std::mem;
use std::sync::Arc;
use {PEER_ID, bencode, amy};
use self::writer::Writer;
use self::reader::Reader;
use std::collections::HashMap;
use std::net::SocketAddr;
use url::percent_encoding::{percent_encode_byte};
use net2::{TcpBuilder, TcpStreamExt};
use std::net::TcpStream;
use url::Url;

pub struct Announcer {
    reg: Arc<amy::Registrar>,
    connections: HashMap<usize, Tracker>,
}

enum Event {
    DNSResolved(dns::QueryResponse),
    Readable,
    Writable,
}

struct Tracker {
    torrent: usize,
    state: TrackerState,
}

enum TrackerState {
    Error,
    ResolvingDNS { sock: TcpBuilder, req: Vec<u8>, port: u16 },
    Writing { sock: TcpStream, writer: Writer },
    Reading { sock: TcpStream, reader: Reader },
    Complete,
}

impl TrackerState {
    fn new(sock: TcpBuilder, req: Vec<u8>, port: u16 ) -> TrackerState {
        TrackerState::ResolvingDNS { sock, req, port }
    }

    fn handle(&mut self, event: Event) -> Result<()> {
        let s = mem::replace(self, TrackerState::Error);
        mem::replace(self, s.next(event)?);
        Ok(())
    }

    fn next(self, event: Event) -> Result<TrackerState> {
        match (self, event) {
            (TrackerState::ResolvingDNS { sock, req, port }, Event::DNSResolved(r)) => {
                let addr = SocketAddr::new(r.res?, port);
                let s = sock.to_tcp_stream().chain_err(|| ErrorKind::IO)?;
                s.set_nonblocking(true).chain_err(|| ErrorKind::IO)?;
                s.connect(addr).chain_err(|| ErrorKind::IO)?;
                Ok(TrackerState::Writing { sock: s, writer: Writer::new(req) })
            }
            (TrackerState::Writing { mut sock, mut writer }, Event::Writable) => {
                match writer.writable(&mut sock)? {
                    Some(()) => {
                        // reg.reregister(id, &sock, amy::Event::Read).chain_err(|| ErrorKind::IO)?;
                        let r = Reader{};
                        Ok(TrackerState::Reading { sock, reader: r })
                    }
                    None => {
                        Ok(TrackerState::Writing { sock, writer })
                    }
                }
            }
            (s @ TrackerState::Writing { .. }, _) => Ok(s),
            (s @ TrackerState::Reading { .. }, _) => Ok(s),
            _ => bail!("Unknown state transition encountered!")
        }
    }
}

impl Announcer {
    pub fn new(reg: Arc<amy::Registrar>) -> Announcer {
        Announcer { reg, connections: HashMap::new(), }
    }

    pub fn contains(&self, id: usize) -> bool {
        self.connections.contains_key(&id)
    }

    pub fn readable(&mut self, id: usize) -> Option<Response> {
        if let Some(mut trk) = self.connections.get_mut(&id) {
            match trk.state.handle(Event::Readable) {
                Ok(()) => { }
                Err(e) => {
                    return Some((trk.torrent, Err(e)));
                }
            }
        }
        None
    }

    pub fn writable(&mut self, id: usize) -> Option<Response> {
        if let Some(mut trk) = self.connections.get_mut(&id) {
            match trk.state.handle(Event::Writable) {
                Ok(()) => { }
                Err(e) => {
                    return Some((trk.torrent, Err(e)));
                }
            }
        }
        None
    }

    pub fn tick(&mut self) -> Vec<Response> {
        Vec::new()
    }

    pub fn dns_resolved(&mut self, resp: dns::QueryResponse) -> Option<Response> {
        if let Some(mut trk) = self.connections.get_mut(&resp.id) {
            match trk.state.handle(Event::DNSResolved(resp)) {
                Ok(()) => { }
                Err(e) => {
                    return Some((trk.torrent, Err(e)));
                }
            }
        }
        None
    }

    pub fn new_announce(&mut self, req: Announce, url: &Url, dns: &mut dns::Resolver) -> Result<()> {
        let mut http_req = Vec::with_capacity(50);
        // Encode GET req
        http_req.extend_from_slice(b"GET ");

        // Encode the URL:
        http_req.extend_from_slice(url.path().as_bytes());
        // The fact that I have to do this is genuinely depressing.
        // This will be rewritten as a proper http protocol
        // encoder in an event loop.
        http_req.extend_from_slice("?".as_bytes());
        append_query_pair(&mut http_req, "info_hash", &encode_param(&req.hash));
        append_query_pair(&mut http_req, "peer_id", &encode_param(&PEER_ID[..]));
        append_query_pair(&mut http_req, "uploaded", &req.uploaded.to_string());
        append_query_pair(&mut http_req, "downloaded", &req.downloaded.to_string());
        append_query_pair(&mut http_req, "left", &req.left.to_string());
        append_query_pair(&mut http_req, "compact", "1");
        append_query_pair(&mut http_req, "port", &req.port.to_string());
        match req.event {
            Some(tracker::Event::Started) => {
                append_query_pair(&mut http_req, "numwant", "50");
                append_query_pair(&mut http_req, "event", "started");
            }
            Some(tracker::Event::Stopped) => {
                append_query_pair(&mut http_req, "event", "started");
            }
            Some(tracker::Event::Completed) => {
                append_query_pair(&mut http_req, "numwant", "20");
                append_query_pair(&mut http_req, "event", "completed");
            }
            None => {
                append_query_pair(&mut http_req, "numwant", "20");
            }
        }

        // Encode HTTP protocol
        http_req.extend_from_slice(b" HTTP/1.1\r\n");
        // Encode host header
        http_req.extend_from_slice(b"Host: ");
        let host = url.host_str().ok_or::<Error>(
            ErrorKind::InvalidRequest(format!("Tracker announce url has no host!")).into()
        )?;
        let port = url.port().unwrap_or(80);
        http_req.extend_from_slice(host.as_bytes());
        http_req.extend_from_slice(b"\r\n");
        // Encode empty line to terminate request
        http_req.extend_from_slice(b"\r\n");
        let sock = TcpBuilder::new_v4().chain_err(|| ErrorKind::IO)?;
        let id = self.reg.register(&sock, amy::Event::Both).chain_err(|| ErrorKind::IO)?;
        dns.new_query(id, host);
        self.connections.insert(id, Tracker {
            torrent: req.id,
            state: TrackerState::new(sock, http_req, port),
        });

        Ok(())
    }
    //     let content = bencode::decode(&mut resp).map_err(
    //         |_| TrackerError::InvalidResponse("HTTP Response must be valid BENcode")
    //     )?;
    //     TrackerResponse::from_bencode(content)
    // }
}

fn append_query_pair(s: &mut Vec<u8>, k: &str, v: &str) {
    s.extend_from_slice(k.as_bytes());
    s.extend_from_slice("=".as_bytes());
    s.extend_from_slice(v.as_bytes());
    s.extend_from_slice("&".as_bytes());
}

fn encode_param(data: &[u8]) -> String {
    let mut resp = String::new();
    for byte in data {
        resp.push_str(percent_encode_byte(*byte));
    }
    resp
}
