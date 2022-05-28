use std::io;
use std::mem;

use crate::tracker::errors::{ErrorKind, Result};
use crate::util::{aread, IOR};

pub struct Reader {
    data: Vec<u8>,
    idx: usize,
    state: ReadState,
}

pub enum ReadRes {
    None,
    Done(Vec<u8>),
    Redirect(String),
}

enum ReadState {
    Header,
    Body,
}

impl Reader {
    pub fn new() -> Reader {
        Reader {
            data: vec![0; 75],
            idx: 0,
            state: ReadState::Header,
        }
    }

    pub fn readable<R: io::Read>(&mut self, conn: &mut R) -> Result<ReadRes> {
        loop {
            match aread(&mut self.data[self.idx..], conn) {
                IOR::Complete => {
                    self.idx = self.data.len();
                    let new_len = (self.idx as f32 * 1.5) as usize;
                    self.data.resize(new_len, 0u8);
                    if let Some(result) = self.process_data()? {
                        return Ok(result);
                    }
                }
                IOR::Incomplete(a) => {
                    self.idx += a;
                    if let Some(result) = self.process_data()? {
                        return Ok(result);
                    }
                }
                IOR::Blocked => return Ok(ReadRes::None),
                IOR::EOF => match self.state {
                    ReadState::Body => {
                        let mut data = mem::replace(&mut self.data, Vec::with_capacity(0));
                        data.truncate(self.idx);
                        return Ok(ReadRes::Done(data));
                    }
                    _ => return Err(ErrorKind::EOF.into()),
                },
                IOR::Err(_) => return Err(ErrorKind::IO.into()),
            }
        }
    }

    fn process_data(&mut self) -> Result<Option<ReadRes>> {
        let mut header_done = None;
        match self.state {
            ReadState::Header => {
                let mut headers = [httparse::EMPTY_HEADER; 32];
                let mut resp = httparse::Response::new(&mut headers);
                match resp.parse(&self.data[..self.idx]) {
                    Ok(httparse::Status::Complete(i)) => {
                        // Redirect handling
                        let redirect_codes = [301, 302, 303, 307, 308];
                        if resp
                            .code
                            .as_ref()
                            .map(|c| redirect_codes.contains(c))
                            .unwrap_or(false)
                        {
                            let loc = resp
                                .headers
                                .iter()
                                .find(|h| h.name == "Location")
                                .and_then(|h| String::from_utf8(h.value.to_vec()).ok());
                            if loc.is_none() {
                                return Err(ErrorKind::InvalidResponse("malformed HTTP").into());
                            }
                            return Ok(Some(ReadRes::Redirect(loc.unwrap())));
                        }
                        header_done = Some(i);
                    }
                    Ok(httparse::Status::Partial) => {}
                    Err(_) => {
                        return Err(ErrorKind::InvalidResponse("malformed HTTP").into());
                    }
                }
            }
            ReadState::Body => {}
        }
        if let Some(i) = header_done {
            let body = self.data.split_off(i);
            self.idx -= self.data.len();
            self.data = body;
            self.state = ReadState::Body;
        }
        Ok(None)
    }
}
