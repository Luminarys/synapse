use chrono::{DateTime, Utc};

use super::criterion::{Criterion, Operation, Value, ResourceKind, Filter, match_n, match_f,
                       match_s, match_b};

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Clone, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(untagged)]
pub enum SResourceUpdate<'a> {
    Resource(&'a Resource),
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
    TorrentPicker { id: u64, sequential: bool },
    PeerRate {
        id: u64,
        rate_up: u32,
        rate_down: u32,
    },
    PieceAvailable { id: u64, available: bool },
    PieceDownloaded { id: u64, downloaded: bool },
}

/// Collection of mutable fields that clients
/// can modify. Due to shared field names, all fields are aggregated
#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub id: u64,
    pub rate_up: u32,
    pub rate_down: u32,
    pub throttle_up: u32,
    pub throttle_down: u32,
    pub started: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub id: u64,
    pub torrent_id: u64,
    pub available: bool,
    pub downloaded: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub id: u64,
    pub torrent_id: u64,
    pub path: String,
    pub progress: f32,
    pub availability: f32,
    pub priority: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tracker {
    pub id: u64,
    pub torrent_id: u64,
    pub url: String,
    pub last_report: DateTime<Utc>,
    pub error: Option<String>
}

// TODO: Consider how to handle datetime matching
// TODO: Proc macros to remove this shit

impl Filter for Resource {
    fn matches(&self, c: &Criterion) -> bool {
        match (self, &c.kind) {
            (&Resource::Server(ref t), &ResourceKind::Server) => t.matches(c),
            (&Resource::Torrent(ref t), &ResourceKind::Torrent) => t.matches(c),
            (&Resource::File(ref t), &ResourceKind::File) => t.matches(c),
            (&Resource::Piece(ref t), &ResourceKind::Piece) => t.matches(c),
            (&Resource::Peer(ref t), &ResourceKind::Peer) => t.matches(c),
            (&Resource::Tracker(ref t), &ResourceKind::Tracker) => t.matches(c),
            _ => false,
        }
    }
}

impl Filter for Server {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "rate_up" => match_n(self.rate_up as u64, c),
            "rate_down" => match_n(self.rate_down as u64, c),
            "throttle_up" => match_n(self.throttle_up as u64, c),
            "throttle_down" => match_n(self.throttle_down as u64, c),

            _ => false,
        }
    }
}

impl Filter for Torrent {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "priority" => match_n(self.priority as u64, c),
            "rate_up" => match_n(self.rate_up as u64, c),
            "rate_down" => match_n(self.rate_down as u64, c),
            "throttle_up" => match_n(self.throttle_up as u64, c),
            "throttle_down" => match_n(self.throttle_down as u64, c),
            "transferred_up" => match_n(self.transferred_up as u64, c),
            "transferred_down" => match_n(self.transferred_down as u64, c),
            "peers" => match_n(self.peers as u64, c),
            "trackers" => match_n(self.trackers as u64, c),
            "pieces" => match_n(self.pieces as u64, c),
            "piece_size" => match_n(self.piece_size as u64, c),
            "files" => match_n(self.files as u64, c),

            "progress" => match_f(self.progress, c),
            "availability" => match_f(self.availability, c),

            "name" => match_s(&self.name, c),
            "path" => match_s(&self.path, c),
            "status" => match_s(self.status.as_str(), c),
            "error" => match_s(self.error.as_ref().map(|s| s.as_str()).unwrap_or(""), c),

            "sequential" => match_b(self.sequential, c),

            _ => false,
        }
    }
}

impl Filter for Piece {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "torrent_id" => match_n(self.id, c),

            "available" => match_b(self.available, c),
            "downloaded" => match_b(self.downloaded, c),

            _ => false,
        }
    }
}

impl Filter for File {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "torrent_id" => match_n(self.id, c),
            "priority" => match_n(self.priority as u64, c),

            "progress" => match_f(self.progress, c),

            "path" => match_s(&self.path, c),

            _ => false,
        }
    }
}

impl Filter for Peer {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "torrent_id" => match_n(self.id, c),
            "rate_up" => match_n(self.rate_up as u64, c),
            "rate_down" => match_n(self.rate_down as u64, c),

            "availability" => match_f(self.availability, c),

            "ip" => match_s(&self.ip, c),

            // TODO: Come up with a way to match this
            "client_id" => false,

            _ => false,
        }
    }
}

impl Filter for Tracker {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_n(self.id, c),
            "torrent_id" => match_n(self.id, c),

            "url" => match_s(&self.url, c),
            "error" => match_s(self.error.as_ref().map(|s| s.as_str()).unwrap_or(""), c),

            _ => false,
        }
    }
}

impl Status {
    pub fn as_str(&self) -> &'static str {
        match *self {
            Status::Pending => "pending",
            Status::Paused => "paused",
            Status::Leeching => "leeching",
            Status::Idle => "idle",
            Status::Seeding => "seeding",
            Status::Hashing => "hashing",
            Status::Error => "error",
        }
    }
}
