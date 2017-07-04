mod reader;
mod writer;

use tracker::{Announce, Response, Event, Result, ErrorKind};
use std::time::Duration;
use util::{encode_param, append_pair};
use std::rc::Rc;
use {PEER_ID, bencode, amy};
use self::writer::Writer;
use std::collections::HashMap;
use socket::Socket;

pub struct Announcer {
    reg: Rc<amy::Registrar>,
    connections: HashMap<usize, Tracker>,
}

struct Tracker {
    torrent: usize,
    writer: Writer,
    sock: Socket,
}

impl Announcer {
    pub fn new(reg: Rc<amy::Registrar>) -> Announcer {
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

    pub fn new_announce(&mut self, req: Announce) -> Option<Result<()>> {
        None
    }
    // pub fn announce(&mut self, mut req: Announce) -> Result<Response> {
    //     let mut url = &mut req.url;
    //     // The fact that I have to do this is genuinely depressing.
    //     // This will be rewritten as a proper http protocol
    //     // encoder in an event loop.
    //     url.push_str("?");
    //     append_pair(&mut url, "info_hash", &encode_param(&req.hash));
    //     append_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
    //     append_pair(&mut url, "uploaded", &req.uploaded.to_string());
    //     append_pair(&mut url, "downloaded", &req.downloaded.to_string());
    //     append_pair(&mut url, "left", &req.left.to_string());
    //     append_pair(&mut url, "compact", "1");
    //     append_pair(&mut url, "port", &req.port.to_string());
    //     match req.event {
    //         Some(Event::Started) => {
    //             append_pair(&mut url, "numwant", "50");
    //             append_pair(&mut url, "event", "started");
    //         }
    //         Some(Event::Stopped) => {
    //             append_pair(&mut url, "event", "started");
    //         }
    //         Some(Event::Completed) => {
    //             append_pair(&mut url, "numwant", "20");
    //             append_pair(&mut url, "event", "completed");
    //         }
    //         None => {
    //             append_pair(&mut url, "numwant", "20");
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
