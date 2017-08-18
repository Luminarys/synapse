use std::collections::HashMap;
use std::net::TcpStream;
use std::io::{self, Read, Write};
use std::{time, thread, fs};

use super::proto::message::Error;
use util::{aread, IOR};

pub struct Transfers {
    torrents: HashMap<usize, TorrentTx>,
}

pub enum TransferResult {
    Torrent {
        conn: TcpStream,
        data: Vec<u8>,
        path: Option<String>,
        client: usize,
        serial: u64,
    },
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
    last_action: time::Instant,
}

const CONN_TIMEOUT: u64 = 2;

impl Transfers {
    pub fn new() -> Transfers {
        Transfers {
            torrents: HashMap::new(),
        }
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
                last_action: time::Instant::now(),
            },
        );
    }

    pub fn add_download(&self, conn: TcpStream, path: String) {
        thread::spawn(move || {
            match handle_dl(conn, path) {
                Ok(()) => {
                }
                Err(_) => {
                    // TODO: ?
                }
            }
        });
    }

    pub fn contains(&self, id: usize) -> bool {
        self.torrents.contains_key(&id)
    }

    pub fn ready(&mut self, id: usize) -> TransferResult {
        match self.torrents.get_mut(&id).map(|tx| tx.readable()) {
            Some(Ok(true)) => {
                let mut tx = self.torrents.remove(&id).unwrap();
                // Send the OK message
                let lines = vec![
                    format!("HTTP/1.1 204 NO CONTENT"),
                    format!("Access-Control-Allow-Origin: {}", "*"),
                    format!("Access-Control-Allow-Methods: {}", "OPTIONS, POST, GET"),
                    format!(
                        "Access-Control-Allow-Headers: {}",
                        "Access-Control-Allow-Headers, Origin, Accept, X-Requested-With, Content-Type, Access-Control-Request-Method, Access-Control-Request-Headers, Authorization"
                    ),
                    format!("Connection: {}", "Close"),
                    format!("\r\n"),
                ];
                let data = lines.join("\r\n");
                if tx.conn.write(data.as_bytes()).is_err() {
                    // Do nothing, we got the data, so who cares.
                }

                TransferResult::Torrent {
                    conn: tx.conn,
                    data: tx.buf,
                    path: tx.path,
                    client: tx.client,
                    serial: tx.serial,
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

    pub fn cleanup(&mut self) -> Vec<(TcpStream, usize, Error)> {
        let mut res = Vec::new();
        let ids: Vec<usize> = self.torrents
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

fn handle_dl(mut conn: TcpStream, path: String) -> io::Result<()> {
    let mut f = fs::File::open(&path)?;
    let len = f.metadata()?.len();

    let lines = vec![
        format!("HTTP/1.1 200 OK"),
        format!("Access-Control-Allow-Origin: {}", "*"),
        format!("Access-Control-Allow-Methods: {}", "OPTIONS, POST, GET"),
        format!(
            "Access-Control-Allow-Headers: {}",
            "Access-Control-Allow-Headers, Origin, Accept, X-Requested-With, Content-Type, Access-Control-Request-Method, Access-Control-Request-Headers, Authorization"
        ),
        format!("Content-Length: {}", len),
        format!("Content-Type: {}", "application/octet-stream"),
        format!("Content-Disposition: attachment; filename=\"{}\"", path),
        format!("Connection: {}", "Close"),
        format!("\r\n"),
    ];
    let data = lines.join("\r\n");
    conn.write_all(data.as_bytes())?;

    let mut buf = vec![0u8; 16384];
    loop {
        let amnt = f.read(&mut buf)?;
        conn.write_all(&buf[0..amnt])?;
        if amnt != buf.len() {
            break;
        }
    }
    Ok(())
}
