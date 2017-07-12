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

    /// Sends a message to a peer
    fn msg_peer(&mut self, peer: PID, msg: torrent::Message);

    /// Sends a message over RPC
    fn msg_rpc(&mut self, msg: rpc::Message);

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
