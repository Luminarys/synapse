use chrono::{DateTime, Utc};

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Torrent(Torrent),
    Piece(Piece),
    File(File),
    Peer(Peer),
    Tracker(Tracker),
}

#[derive(Serialize, Deserialize)]
pub struct Torrent {
    id: u64,
    name: String,
    path: String,
    created: DateTime<Utc>,
    modified: DateTime<Utc>,
    status: Status,
    error: Option<String>,
    priority: u8,
    progress: f32,
    availability: f32,
    sequential: bool,
    rate_up: u32,
    rate_down: u32,
    average_rate_up: f32,
    average_rate_down: f32,
    throttle_up: u32,
    throttle_down: u32,
    transferred_up: u64,
    transferred_down: u64,
    peers: u16,
    trackers: u8,
    pieces: u64,
    files: u32,
}

#[derive(Serialize, Deserialize)]
pub enum Status {
    Pending,
    Leeching,
    Idle,
    Seeding,
    Hashing,
    Error,
}

#[derive(Serialize, Deserialize)]
pub struct Piece {
    id: u64,
    torrent_id: u64,
    available: bool,
    downloaded: bool,
}

#[derive(Serialize, Deserialize)]
pub struct File {
    id: u64,
    torrent_id: u64,
    path: String,
    progress: f32,
    availability: f32,
    priority: u8,
}

#[derive(Serialize, Deserialize)]
pub struct Peer {
    id: u64,
    torrent_id: u64,
    client_id: [u8; 20],
    ip: String,
    rate_up: u32,
    rate_down: u32,
    availability: f32,
}

#[derive(Serialize, Deserialize)]
pub struct Tracker {
    id: u64,
    torrent_id: u64,
    url: String,
    last_report: DateTime<Utc>,
}
