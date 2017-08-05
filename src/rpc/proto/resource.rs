use chrono::{DateTime, Utc};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Resource {
    Torrent(Torrent),
    Piece(Piece),
    File(File),
    Peer(Peer),
    Tracker(Tracker),
}

/// To increase server->client update efficiency, we
/// encode common partial updates to resources with
/// this enum.
#[derive(Serialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum ResourceUpdate {
    Resource(Resource),
    TorrentStatus {
        id: u64,
        error: Option<String>,
        status: Status,
    },
    TorrentTransfer {
        id: u64,
        rate_up: u32,
        rate_down: u32,
        transferred_up: u32,
        transferred_down: u32,
    },
    TorrentPeers {
        id: u64,
        peers: u16,
    },
    PeerRate {
        id: u64,
        rate_up: u32,
        rate_down: u32,
    },
    PieceDownloaded {
        id: u64,
        downloaded: bool,
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    throttle_up: u32,
    throttle_down: u32,
    transferred_up: u64,
    transferred_down: u64,
    peers: u16,
    trackers: u8,
    pieces: u64,
    piece_size: u32,
    files: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Status {
    Pending,
    Leeching,
    Idle,
    Seeding,
    Hashing,
    Error,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    id: u64,
    torrent_id: u64,
    available: bool,
    downloaded: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    id: u64,
    torrent_id: u64,
    path: String,
    progress: f32,
    availability: f32,
    priority: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct Tracker {
    id: u64,
    torrent_id: u64,
    url: String,
    last_report: DateTime<Utc>,
    error: Option<String>,
}
