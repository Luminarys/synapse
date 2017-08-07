use std::collections::HashMap;
use std::net::TcpStream;
use std::io::{Read, Write};

use super::proto::message::Error;
use util::{aread, IOR};

pub struct Transfers {
    torrents: HashMap<usize, TorrentTx>,
}

pub enum TransferResult {
    Torrent { conn: TcpStream, data: Vec<u8>, path: Option<String> },
    Error {
        conn: TcpStream,
        client: usize,
        err: Error,
    },
    Incomplete,
}

struct TorrentTx {
    conn: TcpStream,
    client: usize,
    serial: u64,
    pos: usize,
    buf: Vec<u8>,
    path: Option<String>,
}

impl Transfers {
    pub fn new() -> Transfers {
        Transfers { torrents: HashMap::new() }
    }

    pub fn add_torrent(
        &mut self,
        id: usize,
        client: usize,
        serial: u64,
        conn: TcpStream,
        mut data: Vec<u8>,
        path: Option<String>,
        size: u64,
        ) {
        let pos = data.len();
        data.reserve(size as usize);
        unsafe { data.set_len(size as usize) };
        self.torrents.insert(
            id,
            TorrentTx {
                client,
                serial,
                conn,
                pos,
                buf: data,
                path,
            },
            );
    }

    pub fn contains(&self, id: usize) -> bool {
        self.torrents.contains_key(&id)
    }

    pub fn readable(&mut self, id: usize) -> TransferResult {
        match self.torrents.get_mut(&id).map(|tx| tx.readable()) {
            Some(Ok(true)) => {
                let mut tx = self.torrents.remove(&id).unwrap();
                // Send the OK message
                let lines = vec![
                    format!("HTTP/1.1 204 NO CONTENT"),
                    format!("Access-Control-Allow-Origin: {}", "*"),
                    format!("Access-Control-Allow-Methods: {}", "OPTIONS, POST, GET"),
                    format!("Access-Control-Allow-Headers: {}", "Access-Control-Allow-Headers, Origin, Accept, X-Requested-With, Content-Type, Access-Control-Request-Method, Access-Control-Request-Headers, Authorization"),
                    format!("Connection: Closed"),
                ];
                let data = lines.join("\r\n") + "\r\n\r\n";
                tx.conn.write(data.as_bytes());

                TransferResult::Torrent {
                    conn: tx.conn,
                    data: tx.buf,
                    path: tx.path,
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
}

impl TorrentTx {
    pub fn readable(&mut self) -> Result<bool, &'static str> {
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
}
