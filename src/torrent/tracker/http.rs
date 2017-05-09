use std::io::Cursor;
use mio::tcp::TcpStream;

pub struct HttpTracker {
    conn: TcpStream,
    state: State,
}

enum State {
    Writing { buffer: Cursor<Vec<u8>> },
    Reading { buffer: Cursor<Vec<u8>> },
    Waiting,
}
