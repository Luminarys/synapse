use std::net::{UdpSocket, SocketAddr};
use std::time;
use tracker::{Announce, Result, ResultExt, Response, TrackerResponse, Event, Error, ErrorKind, dns};
use std::collections::HashMap;
use {CONFIG, PEER_ID, amy};
use util::bytes_to_addr;
use std::io::{self, Write, Read, Cursor};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use slog::Logger;
use url::Url;
use rand::random;

// We're not going to bother with backoff, if the tracker/network aren't working now
// the torrent can just resend a request later.
const TIMEOUT_MS: u64 = 5000;
const RETRANS_MS: u64 = 500;
const MAGIC_NUM: u64 = 0x41727101980;

pub struct Handler {
    id: usize,
    sock: UdpSocket,
    connections: HashMap<usize, Connection>,
    transactions: HashMap<u32, usize>,
    conn_count: usize,
    l: Logger,
    buf: Vec<u8>,
}

struct Connection {
    torrent: usize,
    last_updated: time::Instant,
    last_retrans: time::Instant,
    state: State,
    announce: Announce,
}

enum State {
    ResolvingDNS { port: u16 },
    Connecting { addr: SocketAddr, data: [u8; 16] },
    Announcing { addr: SocketAddr, data: [u8; 98] },
}

impl Handler {
    pub fn new(reg: &amy::Registrar, l: Logger) -> io::Result<Handler> {
        let port = CONFIG.port;
        let sock = UdpSocket::bind(("0.0.0.0", port))?;
        sock.set_nonblocking(true)?;
        let id = reg.register(&sock, amy::Event::Read)?;
        Ok(Handler {
            id,
            sock,
            connections: HashMap::new(),
            transactions: HashMap::new(),
            l,
            conn_count: 0,
            buf: vec![0u8; 350],
        })
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn complete(&self) -> bool {
        self.connections.is_empty()
    }

    pub fn contains(&self, id: usize) -> bool {
        self.connections.contains_key(&id)
    }

    pub fn new_announce(
        &mut self,
        req: Announce,
        url: &Url,
        dns: &mut dns::Resolver,
    ) -> Result<()> {
        // TODO: Attempt to parse into an IP address first, then perform dns res
        debug!(self.l, "Received a new announce req for {:?}", url);
        let host = url.host_str().ok_or::<Error>(
            ErrorKind::InvalidRequest(
                format!("Tracker announce url has no host!"),
            ).into(),
        )?;
        let port = url.port().ok_or::<Error>(
            ErrorKind::InvalidRequest(
                format!("Tracker announce url has no port!"),
            ).into(),
        )?;

        let id = self.new_conn();
        self.connections.insert(
            id,
            Connection {
                torrent: req.id,
                last_updated: time::Instant::now(),
                last_retrans: time::Instant::now(),
                state: State::ResolvingDNS { port },
                announce: req,
            },
        );
        debug!(self.l, "Dispatching DNS req for {:?}, url: {:?}", id, host);
        dns.new_query(id, host);
        Ok(())
    }

    pub fn dns_resolved(&mut self, resp: dns::QueryResponse) -> Option<Response> {
        let id = resp.id;
        let mut success = false;
        debug!(self.l, "Received a DNS resp for {:?}", id);
        let resp = if let Some(mut conn) = self.connections.get_mut(&id) {
            match conn.state {
                State::ResolvingDNS { port } => {
                    conn.last_updated = time::Instant::now();
                    let tid = random::<u32>();
                    let mut data = [0u8; 16];
                    {
                        let mut connect_req = Cursor::new(&mut data[..]);
                        connect_req.write_u64::<BigEndian>(MAGIC_NUM).unwrap();
                        connect_req.write_u32::<BigEndian>(0).unwrap();
                        connect_req.write_u32::<BigEndian>(tid).unwrap();
                    }
                    match resp.res {
                        Ok(ip) => {
                            success = true;
                            conn.state = State::Connecting {
                                addr: SocketAddr::new(ip, port),
                                data,
                            };
                            self.transactions.insert(tid, id);
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
            resp
        } else if success {
            self.send_data(id)
        } else {
            None
        }
    }

    pub fn readable(&mut self) -> Vec<Response> {
        let mut resps = Vec::new();
        loop {
            match self.sock.recv_from(&mut self.buf[..]) {
                Ok((v, _)) => {
                    let action = (&self.buf[0..4]).read_u32::<BigEndian>().unwrap();
                    match action {
                        0 if v == 16 => {
                            if let Some(r) = self.process_connect() {
                                resps.push(r);
                            }
                        }
                        1 if v >= 20 => {
                            if let Some(r) = self.process_announce(v) {
                                resps.push(r);
                            }
                        }
                        3 if v >= 8 => {
                            if let Some(r) = self.process_error(v) {
                                resps.push(r);
                            }
                        }
                        _ => debug!(self.l, "Received invalid response from tracker!"),
                    }
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::WouldBlock {
                        break;
                    } else {
                        // TODO: Handle this, could be some odd sort of failure
                        break;
                    }
                }
            }
        }
        resps
    }

    pub fn tick(&mut self) -> Vec<Response> {
        let mut resps = Vec::new();
        let mut retrans = Vec::new();
        {
            let ref l = self.l;

            self.connections.retain(
                |id, conn| if conn.last_updated.elapsed() >
                    time::Duration::from_millis(
                        TIMEOUT_MS,
                    )
                {
                    resps.push((conn.torrent, Err(ErrorKind::Timeout.into())));
                    debug!(l, "Announce {:?} timed out", id);
                    false
                } else {
                    if conn.last_retrans.elapsed() > time::Duration::from_millis(RETRANS_MS) {
                        debug!(l, "Retransmiting req {:?}", id);
                        retrans.push(*id);
                    }
                    true
                },
            );

            let c = &self.connections;
            self.transactions.retain(|_, id| c.contains_key(id));
        }

        for id in retrans {
            if let Some(r) = self.send_data(id) {
                resps.push(r)
            }
        }
        resps
    }

    fn process_connect(&mut self) -> Option<Response> {
        let (transaction_id, connection_id) = {
            let mut connect_resp = Cursor::new(&self.buf[4..16]);
            let tid = connect_resp.read_u32::<BigEndian>().unwrap();
            let cid = connect_resp.read_u64::<BigEndian>().unwrap();
            (tid, cid)
        };

        let id = match self.transactions.remove(&transaction_id) {
            Some(id) => id,
            None => return None,
        };

        let mut data = [0u8; 98];
        {
            let conn = match self.connections.get_mut(&id) {
                Some(conn) => conn,
                None => return None,
            };
            let addr = match conn.state {
                State::Connecting { addr, .. } => addr,
                _ => return None,
            };

            {
                let mut announce_req = Cursor::new(&mut data[..]);
                announce_req.write_u64::<BigEndian>(connection_id).unwrap();
                // announce action
                announce_req.write_u32::<BigEndian>(1).unwrap();

                let tid = random::<u32>();
                announce_req.write_u32::<BigEndian>(tid).unwrap();
                self.transactions.insert(tid, id);

                announce_req.write_all(&conn.announce.hash).unwrap();
                announce_req.write_all(&PEER_ID[..]).unwrap();
                announce_req
                    .write_u64::<BigEndian>(conn.announce.downloaded as u64)
                    .unwrap();
                announce_req
                    .write_u64::<BigEndian>(conn.announce.left as u64)
                    .unwrap();
                announce_req
                    .write_u64::<BigEndian>(conn.announce.uploaded as u64)
                    .unwrap();
                match conn.announce.event {
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
                // Key - TODO: randomly generate this
                announce_req.write_u32::<BigEndian>(0xF00BA).unwrap();
                // Num want
                let nw = conn.announce.num_want.map(|nw| nw as i32).unwrap_or(-1);
                announce_req.write_i32::<BigEndian>(nw).unwrap();
                // port
                announce_req.write_u16::<BigEndian>(conn.announce.port).unwrap();
            }
            conn.state = State::Announcing { addr, data };
            conn.last_updated = time::Instant::now();
        }
        self.send_data(id)
    }

    fn process_announce(&mut self, len: usize) -> Option<Response> {
        let mut announce_resp = Cursor::new(&self.buf[4..len]);
        let mut resp = TrackerResponse::empty();
        let transaction_id = announce_resp.read_u32::<BigEndian>().unwrap();

        let id = match self.transactions.remove(&transaction_id) {
            Some(id) => id,
            None => return None,
        };

        let conn = match self.connections.remove(&id) {
            Some(c) => c,
            None => return None,
        };

        resp.interval = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.leechers = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.seeders = announce_resp.read_u32::<BigEndian>().unwrap();
        if len > 20 {
            let pos = announce_resp.position() as usize;
            for p in announce_resp.get_ref()[pos..].chunks(6) {
                resp.peers.push(bytes_to_addr(p));
            }
        }
        Some((conn.torrent, Ok(resp)))
    }

    fn process_error(&mut self, len: usize) -> Option<Response> {
        let mut s = String::new();
        let mut connect_resp = Cursor::new(&self.buf[4..len]);
        let transaction_id = connect_resp.read_u32::<BigEndian>().unwrap();

        let id = match self.transactions.remove(&transaction_id) {
            Some(id) => id,
            None => return None,
        };

        let conn = match self.connections.remove(&id) {
            Some(c) => c,
            None => return None,
        };

        if connect_resp.read_to_string(&mut s).is_err() {
            Some((
                conn.torrent,
                Err(
                    ErrorKind::InvalidResponse(
                        "Tracker error response was invalid UTF8",
                    ).into(),
                ),
            ))
        } else {
            Some((conn.torrent, Err(ErrorKind::TrackerError(s).into())))
        }
    }

    fn new_conn(&mut self) -> usize {
        let c = self.conn_count;
        self.conn_count = self.conn_count.wrapping_add(1);
        c
    }

    fn send_data(&mut self, id: usize) -> Option<Response> {
        let tid;
        let res = {
            let conn = self.connections.get_mut(&id).unwrap();
            tid = conn.torrent;
            // If this actually blocks, something is really fucked(prob with the NIC)
            // and i dont think we need to care
            match conn.state {
                State::Connecting { ref addr, ref data } => {
                    conn.last_retrans = time::Instant::now();
                    self.sock.send_to(data, addr).chain_err(|| ErrorKind::IO)
                }
                State::Announcing { ref addr, ref data } => {
                    conn.last_retrans = time::Instant::now();
                    self.sock.send_to(data, addr).chain_err(|| ErrorKind::IO)
                }
                _ => Ok(0),
            }
        };

        match res {
            Err(e) => {
                self.connections.remove(&id);
                Some((tid, Err(e)))
            }
            Ok(_) => None,
        }
    }
}
