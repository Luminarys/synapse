use {rpc, tracker, disk, listener, torrent};

error_chain! {
    errors {
        IO {
            description("IO error")
                display("IO error")
        }

        Channel(r: &'static str) {
            description("Channel error")
                display("Encountered channel error: {}", r)
        }
    }
}

pub type PID = usize;
pub type TID = usize;

pub enum Event {
    Timer(TID),
    Peer { peer: PID, event: Result<torrent::Message> },
    RPC(Result<rpc::Request>),
    Tracker(Result<tracker::Response>),
    Disk(Result<disk::Response>),
    Listener(Result<listener::Message>),
}

/// Control IO trait used as an abstraction boundary between
/// the actual logic of the torrent client and the IO that needs
/// to be done.
pub trait CIO {
    /// Returns events for peers, timers, channels, etc.
    fn poll(&mut self, events: &mut Vec<Event>);

    /// Adds a peer to be polled on
    fn add_peer(&mut self, peer: torrent::PeerConn) -> Result<PID>;

    /// Removes a peer
    fn remove_peer(&mut self, peer: PID);

    /// Flushes events on the given vec of peers
    fn flush_peers(&mut self, peers: Vec<PID>);

    /// Sends a message to a peer
    fn msg_peer(&mut self, peer: PID, msg: torrent::Message);

    /// Sends a message over RPC
    fn msg_rpc(&mut self, msg: rpc::CMessage);

    /// Sends a message over RPC
    fn msg_trk(&mut self, msg: tracker::Request);

    /// Sends a message to the disk worker
    fn msg_disk(&mut self, msg: disk::Request);

    /// Sends a message to the listener worker
    fn msg_listener(&mut self, msg: listener::Request);

    /// Sets a timer in milliseconds
    fn set_timer(&mut self, interval: usize) -> Result<TID>;

    /// Creates a copy of the IO object, which has the same underlying data
    fn new_handle(&self) -> Self;
}

#[cfg(test)]
pub mod test {
    use super::{CIO, PID, TID, Event, Result};
    use {rpc, tracker, disk, listener, torrent};
    use std::collections::HashMap;

    /// A reference CIO implementation which serves as a test mock
    pub struct TCIO {
        pub peers: HashMap<PID, torrent::PeerConn>,
        pub peer_msgs: Vec<(PID, torrent::Message)>,
        pub flushed_peers: Vec<PID>,
        pub rpc_msgs: Vec<rpc::CMessage>,
        pub trk_msgs: Vec<tracker::Request>,
        pub disk_msgs: Vec<disk::Request>,
        pub listener_msgs: Vec<listener::Request>,
        pub timers: usize,
        pub peer_cnt: usize,
    }

    impl TCIO {
        pub fn new() -> TCIO {
            TCIO {
                peers: HashMap::new(),
                peer_msgs: Vec::new(),
                flushed_peers: Vec::new(),
                rpc_msgs: Vec::new(),
                trk_msgs: Vec::new(),
                disk_msgs: Vec::new(),
                listener_msgs: Vec::new(),
                timers: 0,
                peer_cnt: 0,
            }
        }
    }

    impl CIO for TCIO {
        fn poll(&mut self, _: &mut Vec<Event>) {
            return;
        }

        fn add_peer(&mut self, peer: torrent::PeerConn) -> Result<PID> {
            let id = self.peer_cnt;
            self.peers.insert(id, peer);
            self.peer_cnt += 1;
            Ok(id)
        }

        fn remove_peer(&mut self, peer: PID) {
            self.peers.remove(&peer);
        }

        fn flush_peers(&mut self, mut peers: Vec<PID>) {
            self.flushed_peers.extend(peers.drain(..));
        }

        fn msg_peer(&mut self, peer: PID, msg: torrent::Message) {
            self.peer_msgs.push((peer, msg));
        }

        fn msg_rpc(&mut self, msg: rpc::CMessage) {
            self.rpc_msgs.push(msg);
        }

        fn msg_trk(&mut self, msg: tracker::Request) {
            self.trk_msgs.push(msg);
        }

        fn msg_disk(&mut self, msg: disk::Request) {
            self.disk_msgs.push(msg);
        }

        fn msg_listener(&mut self, msg: listener::Request) {
            self.listener_msgs.push(msg);
        }

        fn set_timer(&mut self, _: usize) -> Result<TID> {
            let timer = self.timers;
            self.timers += 1;
            Ok(timer)
        }

        fn new_handle(&self) -> Self {
            TCIO::new()
        }
    }

}
