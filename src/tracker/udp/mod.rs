use std::net::{UdpSocket, SocketAddr, SocketAddrV4, Ipv4Addr};
use std::time;
use tracker::{Announce, Result, Response, TrackerResponse, Event, Error, ErrorKind, dns};
use std::collections::HashMap;
use {CONFIG, PEER_ID, amy};
use std::sync::Arc;
use std::io::{self, Write, Read, Cursor};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use slog::Logger;
use url::Url;
use rand::random;

pub struct Handler {
    id: usize,
    sock: UdpSocket,
    connections: HashMap<usize, Connection>,
    conn_count: usize,
    l: Logger,
}

struct Connection {
    torrent: usize,
    last_updated: time::Instant,
    state: State,
    announce: Announce,
}

enum State {
    Error,
    ResolvingDNS { port: u16 },
    Connecting { addr: SocketAddr, data: [u8; 16] },
    Announcing { addr: SocketAddr, tid: u32, cid: u64 },
}

impl Handler {
    pub fn new(reg: &Arc<amy::Registrar>, l: Logger) -> io::Result<Handler> {
        let port = CONFIG.port;
        let sock = UdpSocket::bind(("0.0.0.0", port))?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;
        Ok(Handler {
            id,
            sock,
            connections: HashMap::new(),
            l,
            conn_count: 0,
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn complete(&self) -> bool {
        false
    }

    pub fn new_announce(&mut self, req: Announce, url: &Url, dns: &mut dns::Resolver) -> Result<()> {
        debug!(self.l, "Received a new announce req for {:?}", url);
        let host = url.host_str().ok_or::<Error>(
            ErrorKind::InvalidRequest(format!("Tracker announce url has no host!")).into()
        )?;
        let port = url.port().ok_or::<Error>(
            ErrorKind::InvalidRequest(format!("Tracker announce url has no port!")).into()
        )?;

        let id = self.new_conn();
        self.connections.insert(id, Connection {
            torrent: req.id,
            last_updated: time::Instant::now(),
            state: State::ResolvingDNS { port },
            announce: req,
        });
        debug!(self.l, "Dispatching DNS req for {:?}", id);
        dns.new_query(id, host);
        Ok(())
    }

    pub fn dns_resolved(&mut self, resp: dns::QueryResponse) -> Option<Response> {
        let id = resp.id;
        debug!(self.l, "Received a DNS resp for {:?}", id);
        let resp = if let Some(mut conn) = self.connections.get_mut(&id) {
            match conn.state {
                State::ResolvingDNS { port } => {
                    conn.last_updated = time::Instant::now();
                    let tid = random::<u32>();
                    let mut data = [0u8; 16];
                    {
                        let mut connect_req = Cursor::new(&mut data[..]);
                        connect_req.write_u64::<BigEndian>(0x41727101980).unwrap();
                        connect_req.write_u32::<BigEndian>(0).unwrap();
                        connect_req.write_u32::<BigEndian>(tid).unwrap();
                    }
                    match resp.res {
                        Ok(ip) => {
                            conn.state = State::Connecting { addr: SocketAddr::new(ip, port), data };
                            None
                        }
                        Err(e) => Some((conn.torrent, Err(e))),
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if resp.is_some() {
            self.connections.remove(&id);
        }
        resp
    }

    pub fn readable(&mut self, id: usize) -> Option<Response> {
        None
    }

    pub fn tick(&mut self) -> Vec<Response> {
        Vec::new()
    }

    fn new_conn(&mut self) -> usize {
        let c = self.conn_count;
        self.conn_count += 1;
        c
    }

    fn send_data(&mut self, id: usize) -> Option<Response> {
        None
    }

    /*
    pub fn announce(&mut self, req: Announce) -> TrackerRes {
        let mut data = [0u8; 16];
        let tid = 420;
        {
            let mut connect_req = Cursor::new(&mut data[..]);
            connect_req.write_u64::<BigEndian>(0x41727101980).unwrap();
            connect_req.write_u32::<BigEndian>(0).unwrap();
            connect_req.write_u32::<BigEndian>(tid).unwrap();
        }
        let url = Url::parse(&req.url).unwrap();
        self.sock.send_to(&data, (url.host_str().unwrap(), url.port().unwrap())).map_err(
            |_| TrackerError::ConnectionFailure
        )?;

        let mut data = [0u8; 50];
        let (read, _) = self.sock.recv_from(&mut data).map_err(
            |_| TrackerError::ConnectionFailure
        )?;
        if read < 8 {
            return Err(TrackerError::InvalidResponse("UDP connection response must be at least 8 bytes!"));
        }
        let mut connect_resp = Cursor::new(&data[..]);
        let action_resp = connect_resp.read_u32::<BigEndian>().unwrap();
        let transaction_id = connect_resp.read_u32::<BigEndian>().unwrap();
        if transaction_id != tid {
            return Err(TrackerError::InvalidResponse("Invalid transaction ID in tracker response!"));
        }
        if action_resp == 3 {
            let mut s = String::new();
            connect_resp.read_to_string(&mut s).map_err(
                |_| TrackerError::InvalidResponse("Tracker error response must be UTF8!")
            )?;
            return Err(TrackerError::Error(s));
        }
        let connection_id = connect_resp.read_u64::<BigEndian>().unwrap();

        let mut data = [0u8; 98];
        {
            let mut announce_req = Cursor::new(&mut data[..]);
            announce_req.write_u64::<BigEndian>(connection_id).unwrap();
            announce_req.write_u32::<BigEndian>(1).unwrap();
            announce_req.write_u32::<BigEndian>(transaction_id).unwrap();
            announce_req.write_all(&req.hash).unwrap();
            announce_req.write_all(&PEER_ID[..]).unwrap();
            announce_req.write_u64::<BigEndian>(req.downloaded as u64).unwrap();
            announce_req.write_u64::<BigEndian>(req.left as u64).unwrap();
            announce_req.write_u64::<BigEndian>(req.uploaded as u64).unwrap();
            match req.event {
                Some(Event::Started) => {
                    announce_req.write_u32::<BigEndian>(2).unwrap();
                }
                Some(Event::Stopped) => {
                    announce_req.write_u32::<BigEndian>(3).unwrap();
                }
                Some(Event::Completed) => {
                    announce_req.write_u32::<BigEndian>(1).unwrap();
                }
                None => {
                    announce_req.write_u32::<BigEndian>(0).unwrap();
                }
            }
            // IP
            announce_req.write_u32::<BigEndian>(0).unwrap();
            // Key
            announce_req.write_u32::<BigEndian>(0).unwrap();
            // Num Want
            announce_req.write_u32::<BigEndian>(30).unwrap();
            // Port
            let port = CONFIG.port;
            announce_req.write_u16::<BigEndian>(port).unwrap();
        };
        self.sock.send_to(&data, (url.host_str().unwrap(), url.port().unwrap())).map_err(
            |_| TrackerError::ConnectionFailure
        )?;

        let mut data = [0u8; 200];
        let (read, _) = self.sock.recv_from(&mut data).map_err(
            |_| TrackerError::ConnectionFailure
        )?;
        if read < 8 {
            return Err(TrackerError::InvalidResponse("UDP connection response must be at least 8 bytes!"));
        }
        let mut announce_resp = Cursor::new(&mut data[..read]);
        let action_resp = announce_resp.read_u32::<BigEndian>().unwrap();
        let mut resp = TrackerResponse::empty();
        let transaction_id = announce_resp.read_u32::<BigEndian>().unwrap();
        if transaction_id != tid {
            return Err(TrackerError::InvalidResponse("Invalid transaction ID in tracker response!"));
        }
        if action_resp == 3 {
            let mut s = String::new();
            connect_resp.read_to_string(&mut s).map_err(
                |_| TrackerError::InvalidResponse("Tracker error response must be UTF8!")
            )?;
            return Err(TrackerError::Error(s));
        }
        resp.interval = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.leechers = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.seeders = announce_resp.read_u32::<BigEndian>().unwrap();
        for p in announce_resp.get_ref()[20..read].chunks(6) {
            let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
            let socket = SocketAddrV4::new(ip, (&p[4..]).read_u16::<BigEndian>().unwrap());
            resp.peers.push(SocketAddr::V4(socket));
        }
        Ok(resp)
    }
        */
}
