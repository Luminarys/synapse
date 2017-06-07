use tracker::{Request, TrackerRes, TrackerResponse, TrackerError, Event};
use std::time::Duration;
use util::{encode_param, append_pair};
use {PEER_ID, reqwest, bencode};

pub struct Announcer {
    client: reqwest::Client,
}

impl Announcer {
    pub fn new() -> Announcer {
        let mut client = reqwest::Client::new().unwrap();
        client.timeout(Duration::new(1, 0));
        Announcer { client: reqwest::Client::new().unwrap() }
    }

    pub fn announce(&mut self, mut req: Request) -> TrackerRes {
        let mut url = &mut req.url;
        // The fact that I have to do this is genuinely depressing.
        // This will be rewritten as a proper http protocol
        // encoder in an event loop.
        url.push_str("?");
        append_pair(&mut url, "info_hash", &encode_param(&req.hash));
        append_pair(&mut url, "peer_id", &encode_param(&PEER_ID[..]));
        append_pair(&mut url, "uploaded", &req.uploaded.to_string());
        append_pair(&mut url, "downloaded", &req.downloaded.to_string());
        append_pair(&mut url, "left", &req.left.to_string());
        append_pair(&mut url, "compact", "1");
        append_pair(&mut url, "port", &req.port.to_string());
        match req.event {
            Some(Event::Started) => {
                append_pair(&mut url, "numwant", "50");
                append_pair(&mut url, "event", "started");
            }
            Some(Event::Stopped) => {
                append_pair(&mut url, "event", "started");
            }
            Some(Event::Completed) => {
                append_pair(&mut url, "numwant", "20");
                append_pair(&mut url, "event", "completed");
            }
            None => {
                append_pair(&mut url, "numwant", "20");
            }
        }
        let mut resp = self.client.get(&*url).send().map_err(
            |_| TrackerError::ConnectionFailure
        )?;
        let content = bencode::decode(&mut resp).map_err(
            |_| TrackerError::InvalidResponse("HTTP Response must be valid BENcode")
        )?;
        TrackerResponse::from_bencode(content)
    }
}
