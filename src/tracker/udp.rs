use std::net::{UdpSocket, SocketAddr, SocketAddrV4, Ipv4Addr};
use std::sync::atomic;
use tracker::{Request, Response, Event};
use {PORT, PEER_ID};
use std::io::{Write, Cursor};
use byteorder::{ReadBytesExt, WriteBytesExt, BigEndian};
use url::Url;

pub struct Announcer {
    sock: UdpSocket,
}

impl Announcer {
    pub fn new() -> Announcer {
        let port = PORT.load(atomic::Ordering::Relaxed) as u16;
        let sock = UdpSocket::bind(("0.0.0.0", port)).unwrap();
        Announcer {
            sock
        }
    }

    pub fn announce(&mut self, req: Request) -> Response {
        let mut data = [0u8; 16];
        {
            let mut connect_req = Cursor::new(&mut data[..]);
            let tid = 420;
            connect_req.write_u64::<BigEndian>(0x41727101980).unwrap();
            connect_req.write_u32::<BigEndian>(0).unwrap();
            connect_req.write_u32::<BigEndian>(tid).unwrap();
        }
        let url = Url::parse(&req.url).unwrap();
        self.sock.send_to(&data, (url.host_str().unwrap(), url.port().unwrap())).unwrap();

        let mut data = [0u8; 16];
        let (read, _) = self.sock.recv_from(&mut data).unwrap();
        let mut connect_resp = Cursor::new(&mut data);
        assert_eq!(read, 16);
        let action_resp = connect_resp.read_u32::<BigEndian>().unwrap();
        assert_eq!(action_resp, 0);
        let transaction_id = connect_resp.read_u32::<BigEndian>().unwrap();
        let connection_id = connect_resp.read_u64::<BigEndian>().unwrap();

        let mut data = [0u8; 98];
        {
            let mut announce_req = Cursor::new(&mut data[..]);
            announce_req.write_u64::<BigEndian>(connection_id).unwrap();
            announce_req.write_u32::<BigEndian>(1).unwrap();
            announce_req.write_u32::<BigEndian>(transaction_id).unwrap();
            announce_req.write(&req.hash).unwrap();
            announce_req.write(&PEER_ID[..]).unwrap();
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
            let port = PORT.load(atomic::Ordering::Relaxed) as u16;
            announce_req.write_u16::<BigEndian>(port).unwrap();
        };
        self.sock.send_to(&data, (url.host_str().unwrap(), url.port().unwrap())).unwrap();

        let mut data = [0u8; 200];
        let (read, _) = self.sock.recv_from(&mut data).unwrap();
        println!("Read {:?} bytes", read);
        assert!(read >= 20);
        assert!((read - 20) % 6 == 0);
        let mut announce_resp = Cursor::new(&mut data[..read]);
        let action_resp = announce_resp.read_u32::<BigEndian>().unwrap();
        let mut resp = Response::empty(req.id);
        assert_eq!(action_resp, 1);
        assert_eq!(announce_resp.read_u32::<BigEndian>().unwrap(), transaction_id);
        resp.interval = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.leechers = announce_resp.read_u32::<BigEndian>().unwrap();
        resp.seeders = announce_resp.read_u32::<BigEndian>().unwrap();
        for p in announce_resp.get_ref()[20..].chunks(6) {
            let ip = Ipv4Addr::new(p[0], p[1], p[2], p[3]);
            let socket = SocketAddrV4::new(ip, (&p[4..]).read_u16::<BigEndian>().unwrap());
            resp.peers.push(SocketAddr::V4(socket));
        }
        resp
    }
}
