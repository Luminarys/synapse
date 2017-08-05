use chrono::{DateTime, Utc};

use super::criterion::{Criterion, Operation, Value, ResourceKind, Filter};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum Resource {
    Server(Server),
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
pub enum SResourceUpdate {
    Resource(Resource),
    Throttle {
        id: u64,
        throttle_up: u32,
        throttle_down: u32,
    },
    ServerTransfer {
        id: u64,
        rate_up: u32,
        rate_down: u32,
    },
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
        progress: f32,
    },
    TorrentPeers {
        id: u64,
        peers: u16,
        availability: f32,
    },
    TorrentPicker {
        id: u64,
        sequential: bool,
    },
    PeerRate {
        id: u64,
        rate_up: u32,
        rate_down: u32,
    },
    PieceAvailable {
        id: u64,
        available: bool,
    },
    PieceDownloaded {
        id: u64,
        downloaded: bool,
    }
}

/// Collection of mutable fields that clients
/// can modify. Due to shared field names, all fields are aggregated
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CResourceUpdate {
    pub id: u64,
    pub status: Option<Status>,
    pub path: Option<String>,
    pub priority: Option<u8>,
    pub sequential: Option<bool>,
    pub throttle_up: Option<u32>,
    pub throttle_down: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub id: u64,
    pub rate_up: u32,
    pub rate_down: u32,
    pub throttle_up: u32,
    pub throttle_down: u32,
    pub started: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Torrent {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub status: Status,
    pub error: Option<String>,
    pub priority: u8,
    pub progress: f32,
    pub availability: f32,
    pub sequential: bool,
    pub rate_up: u32,
    pub rate_down: u32,
    pub throttle_up: u32,
    pub throttle_down: u32,
    pub transferred_up: u64,
    pub transferred_down: u64,
    pub peers: u16,
    pub trackers: u8,
    pub pieces: u64,
    pub piece_size: u32,
    pub files: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Status {
    Pending,
    Paused,
    Leeching,
    Idle,
    Seeding,
    Hashing,
    Error,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub id: u64,
    pub torrent_id: u64,
    pub available: bool,
    pub downloaded: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub id: u64,
    pub torrent_id: u64,
    pub path: String,
    pub progress: f32,
    pub availability: f32,
    pub priority: u8,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: u64,
    pub torrent_id: u64,
    pub client_id: [u8; 20],
    pub ip: String,
    pub rate_up: u32,
    pub rate_down: u32,
    pub availability: f32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tracker {
    pub id: u64,
    pub torrent_id: u64,
    pub url: String,
    pub last_report: DateTime<Utc>,
    pub error: Option<String>,
}

impl Filter for Torrent {
    fn matches(&self, c: &Criterion) -> bool {
        /*
        match c.field {
            "id" => {
            }
        }
        */
        false
    }
}
