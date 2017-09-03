use std::collections::HashMap;
use std::net::TcpStream;
use std::io::{self, Write};
use std::{time, thread, fs};
use std::path::Path;

use super::proto::message::Error;
use super::EMPTY_HTTP_RESP;

use util::{aread, IOR};

pub struct Transfers {
    torrents: HashMap<usize, TorrentTx>,
}

pub enum TransferResult {
    Torrent {
        conn: TcpStream,
        start: bool,
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
    start: bool,
    path: Option<String>,
    last_action: time::Instant,
}

const CONN_TIMEOUT: u64 = 2;

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
        start: bool,
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
                start,
                last_action: time::Instant::now(),
            },
        );
    }

    pub fn add_download(&self, conn: TcpStream, path: String) {
        // TODO: Transfer this to the disk thread ideally
        thread::spawn(move || match handle_dl(conn, path) {
            Ok(()) => {
                    debug!("DL transfer succeeded");
                }
            Err(e) => {
                debug!("DL transfer failed: {:?}", e);
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

    let p = Path::new(&path);
    let lines = vec![
        format!("HTTP/1.1 200 OK"),
        format!("Content-Length: {}", len),
        format!("Content-Type: {}", "application/octet-stream"),
        format!(
            "Content-Disposition: attachment; filename=\"{}\"",
            p.file_name().unwrap().to_string_lossy()
        ),
        format!("Connection: {}", "Close"),
        format!("\r\n"),
    ];
    let data = lines.join("\r\n");
    conn.write_all(data.as_bytes())?;
    io::copy(&mut f, &mut conn)?;
    Ok(())
}
