mod reader;
mod writer;

use tracker::{Announce, Response, Event, Result, Error, ErrorKind, dns};
use std::time::Duration;
use std::sync::Arc;
use {PEER_ID, bencode, amy};
use self::writer::Writer;
use std::collections::HashMap;
use socket::Socket;
use url::percent_encoding::{percent_encode_byte};
use url::Url;

pub struct Announcer {
    reg: Arc<amy::Registrar>,
    connections: HashMap<usize, Tracker>,
}

struct Tracker {
    torrent: usize,
    writer: Writer,
    sock: Socket,
}

impl Announcer {
    pub fn new(reg: Arc<amy::Registrar>) -> Announcer {
        Announcer { reg, connections: HashMap::new(), }
    }

    pub fn contains(&self, id: usize) -> bool {
        false
    }

    pub fn readable(&mut self, id: usize) -> Option<Response> {
        None
    }
    pub fn writable(&mut self, id: usize) -> Option<Response> {
        if let Some(mut trk) = self.connections.get_mut(&id) {
            match trk.writer.writable(&mut trk.sock) {
                Ok(Some(())) => {
                    self.reg.reregister(id, &trk.sock, amy::Event::Read);
                }
                Ok(None) => {}
                Err(e) => {
                    // return Some((trk.torrent, e.into()));
                }
            }
        }
        None
    }

    pub fn tick(&mut self) -> Vec<Response> {
        Vec::new()
    }

    pub fn dns_resolved(&mut self, resp: dns::QueryResponse) -> Result<()> {
        Ok(())
    }

    pub fn new_announce(&mut self, req: Announce, url: &Url) -> Result<()> {
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
            Some(Event::Started) => {
                append_query_pair(&mut http_req, "numwant", "50");
                append_query_pair(&mut http_req, "event", "started");
            }
            Some(Event::Stopped) => {
                append_query_pair(&mut http_req, "event", "started");
            }
            Some(Event::Completed) => {
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
        http_req.extend_from_slice(host.as_bytes());
        http_req.extend_from_slice(b"\r\n");
        // Encode empty line to terminate request
        http_req.extend_from_slice(b"\r\n");

        // pub fn new(addr: &SocketAddr) -> io::Result<Socket> {
        // let sock = Socket::new();
        Ok(())
    }
    // pub fn announce(&mut self, mut req: Announce) -> Result<Response> {
    //     let mut url = &mut req.url;
    //     // The fact that I have to do this is genuinely depressing.
    //     // This will be rewritten as a proper http protocol
    //     // encoder in an event loop.
    //     url.push_str("?");
    //     append_query_pair(&mut url, "info_hash", &encode_param(&req.hash));
    //     append_query_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
    //     append_query_pair(&mut url, "uploaded", &req.uploaded.to_string());
    //     append_query_pair(&mut url, "downloaded", &req.downloaded.to_string());
    //     append_query_pair(&mut url, "left", &req.left.to_string());
    //     append_query_pair(&mut url, "compact", "1");
    //     append_query_pair(&mut url, "port", &req.port.to_string());
    //     match req.event {
    //         Some(Event::Started) => {
    //             append_query_pair(&mut url, "numwant", "50");
    //             append_query_pair(&mut url, "event", "started");
    //         }
    //         Some(Event::Stopped) => {
    //             append_query_pair(&mut url, "event", "started");
    //         }
    //         Some(Event::Completed) => {
    //             append_query_pair(&mut url, "numwant", "20");
    //             append_query_pair(&mut url, "event", "completed");
    //         }
    //         None => {
    //             append_query_pair(&mut url, "numwant", "20");
    //         }
    //     }
    //     let mut resp = self.client.get(&*url).send().map_err(
    //         |_| TrackerError::ConnectionFailure
    //     )?;
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
