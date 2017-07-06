use std::net::{UdpSocket, SocketAddr, SocketAddrV4, Ipv4Addr};
use tracker::{Announce, Result, Response, Event};
use {CONFIG, PEER_ID, amy};
use std::sync::Arc;
use std::io::{Write, Read, Cursor};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use url::Url;

pub struct Handler {
    sock: UdpSocket,
    reg: Arc<amy::Registrar>,
}

impl Handler {
    pub fn new(reg: Arc<amy::Registrar>) -> Handler {
        let port = CONFIG.port;
        let sock = UdpSocket::bind(("0.0.0.0", port)).unwrap();
        Handler {
            sock, reg
        }
    }

    pub fn contains(&self, id: usize) -> bool {
        false
    }

    pub fn complete(&self) -> bool {
        false
    }

    pub fn readable(&mut self, id: usize) -> Option<Response> {
        None
    }
    pub fn writable(&mut self, id: usize) -> Option<Response> {
        None
    }
    pub fn tick(&mut self) -> Vec<Response> {
        Vec::new()
    }

    pub fn new_announce(&mut self, req: Announce) -> Result<()> {
        Ok(())
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
