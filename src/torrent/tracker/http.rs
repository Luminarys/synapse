use std::io::{self, Cursor, Read, Write};
use socket::Socket;
use torrent::tracker::{Request, Response, Event};
use url::Url;
use util::{append_pair, encode_param, io_err};
use ::PEER_ID;

pub struct HttpTracker {
    conn: Socket,
    state: State,
    url: Url,
}

impl HttpTracker {
    pub fn new() -> HttpTracker {
        unimplemented!();
    }

    pub fn new_request(&mut self, req: Request) -> io::Result<()> {
        let mut enc_req = String::new();
        enc_req.push_str("GET ");
        enc_req.push_str(self.url.path());
        enc_req.push_str("?");
        append_pair(&mut enc_req, "info_hash", &encode_param(&req.hash));
        append_pair(&mut enc_req, "peer_id", &encode_param(&PEER_ID[..]));
        append_pair(&mut enc_req, "uploaded", &req.uploaded.to_string());
        append_pair(&mut enc_req, "numwant", "25");
        append_pair(&mut enc_req, "downloaded", &req.downloaded.to_string());
        append_pair(&mut enc_req, "left", &req.left.to_string());
        append_pair(&mut enc_req, "compact", "1");
        append_pair(&mut enc_req, "port", &req.port.to_string());
        match req.event {
            Event::Started => {
                append_pair(&mut enc_req, "event", "started");
            }
            Event::Stopped => {
                append_pair(&mut enc_req, "event", "stopped");
            }
            Event::Completed => {
                append_pair(&mut enc_req, "event", "completed");
            }
        }
        enc_req.push_str(" HTTP/1.1\r\n");
        enc_req.push_str("Host: ");
        enc_req.push_str(self.url.host_str().unwrap());
        enc_req.push_str("\r\n");
        enc_req.push_str("Connection: close\r\n");
        enc_req.push_str("\r\n\r\n");
        self.state = State::Writing {buf: enc_req.into_bytes(), idx: 0 };
        self.writable()
    }

    pub fn readable(&mut self) -> io::Result<Option<Response>> {
        let res = match self.state {
            State::Reading { ref mut buf, ref mut idx } => {
                let amnt = self.conn.read(&mut buf[*idx..])?;
                *idx += amnt;
                Some(1)
            }
            _ => None
        };
        if let Some(resp) = res {
            self.state = State::Idle;
            Ok(None)
        } else {
            Ok(None)
        }
    }

    pub fn writable(&mut self) -> io::Result<()> {
        // let mut s = mem::replace(&mut self.state, State::Idle);
        let done = match self.state {
            State::Writing { ref buf, ref mut idx } => {
                let amnt = self.conn.write(&buf[*idx..])?;
                if amnt == 0 {
                    // Conn closed
                }
                *idx += amnt;
                if *idx == buf.len() {
                    true
                } else {
                    false
                }
            }
            _ => false
        };
        if done {
            self.state = State::Reading { buf: vec![0u8; 1024], idx: 0 };
        }
        Ok(())
    }
}

enum State {
    Writing { buf: Vec<u8>, idx: usize },
    Reading { buf: Vec<u8>, idx: usize },
    Idle,
}
