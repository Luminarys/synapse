use std::io::Write;
use std::time;

use sstream::SStream;

use super::proto::message::Error;
use super::EMPTY_HTTP_RESP;

use crate::util::{aread, UHashMap, IOR};

pub struct Transfers {
    torrents: UHashMap<TorrentTx>,
}

pub enum TransferResult {
    Torrent {
        conn: SStream,
        start: bool,
        import: bool,
        data: Vec<u8>,
        path: Option<String>,
        client: usize,
        serial: u64,
    },
    Error {
        conn: SStream,
        client: usize,
        err: Error,
    },
    Incomplete,
}

struct TorrentTx {
    conn: SStream,
    client: usize,
    serial: u64,
    pos: usize,
    buf: Vec<u8>,
    start: bool,
    import: bool,
    path: Option<String>,
    last_action: time::Instant,
}

const CONN_TIMEOUT: u64 = 2;

impl Transfers {
    pub fn new() -> Transfers {
        Transfers {
            torrents: UHashMap::default(),
        }
    }

    pub fn add_torrent(
        &mut self,
        id: usize,
        client: usize,
        serial: u64,
        conn: SStream,
        mut data: Vec<u8>,
        path: Option<String>,
        size: u64,
        start: bool,
        import: bool,
    ) {
        let pos = data.len();
        // Given that this requires an authenticated connection
        // we can safely assume this won't be abused
        data.resize(size as usize, 0u8);
        self.torrents.insert(
            id,
            TorrentTx {
                client,
                serial,
                conn,
                pos,
                buf: data,
                path,
                start,
                import,
                last_action: time::Instant::now(),
            },
        );
    }

    pub fn contains(&self, id: usize) -> bool {
        self.torrents.contains_key(&id)
    }

    pub fn ready(&mut self, id: usize) -> TransferResult {
        match self.torrents.get_mut(&id).map(|tx| tx.readable()) {
            Some(Ok(true)) => {
                let mut tx = self.torrents.remove(&id).unwrap();
                if tx.conn.write(&EMPTY_HTTP_RESP).is_err() {
                    // Do nothing, we got the data, so who cares.
                }

                TransferResult::Torrent {
                    conn: tx.conn,
                    data: tx.buf,
                    path: tx.path,
                    client: tx.client,
                    serial: tx.serial,
                    start: tx.start,
                    import: tx.import,
                }
            }
            Some(Ok(false)) => TransferResult::Incomplete,
            Some(Err(e)) => {
                let tx = self.torrents.remove(&id).unwrap();
                TransferResult::Error {
                    conn: tx.conn,
                    client: tx.client,
                    err: Error {
                        serial: Some(tx.serial),
                        reason: e.to_owned(),
                    },
                }
            }
            None => TransferResult::Incomplete,
        }
    }

    pub fn cleanup(&mut self) -> Vec<(SStream, usize, Error)> {
        let mut res = Vec::new();
        let ids: Vec<usize> = self
            .torrents
            .iter()
            .filter(|&(_, ref t)| t.timed_out())
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let tx = self.torrents.remove(&id).unwrap();
            res.push((
                tx.conn,
                id,
                Error {
                    serial: Some(tx.serial),
                    reason: "Timeout".to_owned(),
                },
            ));
        }
        res
    }
}

impl TorrentTx {
    pub fn readable(&mut self) -> Result<bool, &'static str> {
        self.last_action = time::Instant::now();
        loop {
            match aread(&mut self.buf[self.pos..], &mut self.conn) {
                IOR::Complete => return Ok(true),
                IOR::Incomplete(a) => self.pos += a,
                IOR::Blocked => return Ok(false),
                IOR::EOF => return Err("Unexpected EOF!"),
                IOR::Err(_) => return Err("IO error!"),
            }
        }
    }

    pub fn timed_out(&self) -> bool {
        self.last_action.elapsed().as_secs() > CONN_TIMEOUT
    }
}
