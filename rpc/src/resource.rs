use std::mem;
use std::fmt;

use chrono::{DateTime, Utc};

use super::criterion::{Criterion, Filter, match_n, match_f, match_s, match_b};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type")]
#[serde(rename_all = "lowercase")]
pub enum Resource {
    Server(Server),
    Torrent(Torrent),
    Piece(Piece),
    File(File),
    Peer(Peer),
    Tracker(Tracker),
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    Server = 0,
    Torrent,
    Peer,
    File,
    Piece,
    Tracker,
}

/// To increase server->client update efficiency, we
/// encode common partial updates to resources with
/// this enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum SResourceUpdate<'a> {
    #[serde(skip_deserializing)]
    Resource(&'a Resource),
    #[serde(rename = "RESOURCE")]
    OResource(Resource),
    Throttle {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        throttle_up: Option<i64>,
        throttle_down: Option<i64>,
    },
    Rate {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        rate_up: u64,
        rate_down: u64,
    },

    ServerTransfer {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        rate_up: u64,
        rate_down: u64,
        transferred_up: u64,
        transferred_down: u64,
        ses_transferred_up: u64,
        ses_transferred_down: u64,
    },

    TorrentStatus {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        error: Option<String>,
        status: Status,
    },
    TorrentTransfer {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        rate_up: u64,
        rate_down: u64,
        transferred_up: u64,
        transferred_down: u64,
        progress: f32,
    },
    TorrentPeers {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        peers: u16,
        availability: f32,
    },
    TorrentPicker {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        sequential: bool,
    },
    TorrentPriority {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        priority: u8,
    },
    TorrentPath {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        path: String,
    },

    TrackerStatus {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        last_report: DateTime<Utc>,
        error: Option<String>,
    },

    FilePriority {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        priority: u8,
    },
    FileProgress {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        progress: f32,
    },

    PieceAvailable {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        available: bool,
    },
    PieceDownloaded {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        downloaded: bool,
    },
}

/// Collection of mutable fields that clients
/// can modify. Due to shared field names, all fields are aggregated
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CResourceUpdate {
    pub id: String,
    pub path: Option<String>,
    pub priority: Option<u8>,
    pub sequential: Option<bool>,
    #[serde(default)]
    pub throttle_up: Option<Option<i64>>,
    #[serde(default)]
    pub throttle_down: Option<Option<i64>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub id: String,
    pub rate_up: u64,
    pub rate_down: u64,
    pub throttle_up: Option<i64>,
    pub throttle_down: Option<i64>,
    pub transferred_up: u64,
    pub transferred_down: u64,
    pub ses_transferred_up: u64,
    pub ses_transferred_down: u64,
    pub started: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Torrent {
    pub id: String,
    pub name: Option<String>,
    pub path: String,
    pub created: DateTime<Utc>,
    pub modified: DateTime<Utc>,
    pub status: Status,
    pub error: Option<String>,
    pub priority: u8,
    pub progress: f32,
    pub availability: f32,
    pub sequential: bool,
    pub rate_up: u64,
    pub rate_down: u64,
    pub throttle_up: Option<i64>,
    pub throttle_down: Option<i64>,
    pub transferred_up: u64,
    pub transferred_down: u64,
    pub peers: u16,
    pub trackers: u8,
    pub size: Option<u64>,
    pub pieces: Option<u64>,
    pub piece_size: Option<u32>,
    pub files: Option<u32>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub id: String,
    pub torrent_id: String,
    pub available: bool,
    pub downloaded: bool,
    pub index: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub id: String,
    pub torrent_id: String,
    pub path: String,
    pub progress: f32,
    pub availability: f32,
    pub priority: u8,
    pub size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: String,
    pub torrent_id: String,
    pub client_id: String,
    pub ip: String,
    pub rate_up: u64,
    pub rate_down: u64,
    pub availability: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tracker {
    pub id: String,
    pub torrent_id: String,
    pub url: String,
    pub last_report: DateTime<Utc>,
    pub error: Option<String>,
}

impl<'a> SResourceUpdate<'a> {
    pub fn id(&self) -> &str {
        match self {
            &SResourceUpdate::Resource(ref r) => r.id(),
            &SResourceUpdate::OResource(ref r) => r.id(),
            &SResourceUpdate::Throttle { ref id, .. } |
            &SResourceUpdate::Rate { ref id, .. } |
            &SResourceUpdate::ServerTransfer { ref id, .. } |
            &SResourceUpdate::TorrentStatus { ref id, .. } |
            &SResourceUpdate::TorrentTransfer { ref id, .. } |
            &SResourceUpdate::TorrentPeers { ref id, .. } |
            &SResourceUpdate::TorrentPicker { ref id, .. } |
            &SResourceUpdate::TorrentPriority { ref id, .. } |
            &SResourceUpdate::TorrentPath { ref id, .. } |
            &SResourceUpdate::FilePriority { ref id, .. } |
            &SResourceUpdate::FileProgress { ref id, .. } |
            &SResourceUpdate::TrackerStatus { ref id, .. } |
            &SResourceUpdate::PieceAvailable { ref id, .. } |
            &SResourceUpdate::PieceDownloaded { ref id, .. } => id,
        }
    }
}

impl Resource {
    pub fn id(&self) -> &str {
        match self {
            &Resource::Server(ref t) => &t.id,
            &Resource::Torrent(ref t) => &t.id,
            &Resource::File(ref t) => &t.id,
            &Resource::Piece(ref t) => &t.id,
            &Resource::Peer(ref t) => &t.id,
            &Resource::Tracker(ref t) => &t.id,
        }
    }

    pub fn torrent_id(&self) -> Option<&str> {
        match self {
            &Resource::File(ref t) => Some(&t.torrent_id),
            &Resource::Piece(ref t) => Some(&t.torrent_id),
            &Resource::Peer(ref t) => Some(&t.torrent_id),
            &Resource::Tracker(ref t) => Some(&t.torrent_id),
            _ => None,
        }
    }

    pub fn kind(&self) -> ResourceKind {
        match self {
            &Resource::Server(_) => ResourceKind::Server,
            &Resource::Torrent(_) => ResourceKind::Torrent,
            &Resource::File(_) => ResourceKind::File,
            &Resource::Piece(_) => ResourceKind::Piece,
            &Resource::Peer(_) => ResourceKind::Peer,
            &Resource::Tracker(_) => ResourceKind::Tracker,
        }
    }

    pub fn as_server(&self) -> &Server {
        match self {
            &Resource::Server(ref s) => s,
            _ => panic!(),
        }
    }

    pub fn as_torrent(&self) -> &Torrent {
        match self {
            &Resource::Torrent(ref t) => t,
            _ => panic!(),
        }
    }

    pub fn as_file(&self) -> &File {
        match self {
            &Resource::File(ref f) => f,
            _ => panic!(),
        }
    }

    pub fn as_piece(&self) -> &Piece {
        match self {
            &Resource::Piece(ref p) => p,
            _ => panic!(),
        }
    }

    pub fn as_peer(&self) -> &Peer {
        match self {
            &Resource::Peer(ref p) => p,
            _ => panic!(),
        }
    }

    pub fn as_tracker(&self) -> &Tracker {
        match self {
            &Resource::Tracker(ref t) => t,
            _ => panic!(),
        }
    }

    pub fn update(&mut self, update: SResourceUpdate) {
        match (self, update) {
            (&mut Resource::Torrent(ref mut t),
             SResourceUpdate::Throttle {
                 throttle_up,
                 throttle_down,
                 ..
             }) => {
                t.throttle_up = throttle_up;
                t.throttle_down = throttle_down;
            }
            (&mut Resource::Server(ref mut s),
             SResourceUpdate::Throttle {
                 throttle_up,
                 throttle_down,
                 ..
             }) => {
                s.throttle_up = throttle_up;
                s.throttle_down = throttle_down;
            }
            (&mut Resource::Server(ref mut s),
             SResourceUpdate::ServerTransfer {
                 rate_up,
                 rate_down,
                 transferred_up,
                 transferred_down,
                 ses_transferred_up,
                 ses_transferred_down,
                 ..
             }) => {
                s.rate_up = rate_up;
                s.rate_down = rate_down;
                s.transferred_up = transferred_up;
                s.transferred_down = transferred_down;
                s.ses_transferred_up = ses_transferred_up;
                s.ses_transferred_down = ses_transferred_down;
            }
            (&mut Resource::Server(ref mut s),
             SResourceUpdate::Rate { rate_up, rate_down, .. }) => {
                s.rate_up = rate_up;
                s.rate_down = rate_down;
            }
            (&mut Resource::Torrent(ref mut t),
             SResourceUpdate::TorrentStatus {
                 ref mut error,
                 status,
                 ..
             }) => {
                mem::swap(&mut t.error, error);
                t.status = status;
            }
            (&mut Resource::Torrent(ref mut t),
             SResourceUpdate::TorrentTransfer {
                 rate_up,
                 rate_down,
                 transferred_up,
                 transferred_down,
                 progress,
                 ..
             }) => {
                t.rate_up = rate_up;
                t.rate_down = rate_down;
                t.transferred_up = transferred_up;
                t.transferred_down = transferred_down;
                t.progress = progress;
            }
            (&mut Resource::Torrent(ref mut t),
             SResourceUpdate::TorrentPeers {
                 peers,
                 availability,
                 ..
             }) => {
                t.peers = peers;
                t.availability = availability;
            }
            (&mut Resource::Torrent(ref mut t),
             SResourceUpdate::TorrentPicker { sequential, .. }) => {
                t.sequential = sequential;
            }
            (&mut Resource::Peer(ref mut p), SResourceUpdate::Rate { rate_up, rate_down, .. }) => {
                p.rate_up = rate_up;
                p.rate_down = rate_down;
            }
            (&mut Resource::Piece(ref mut p),
             SResourceUpdate::PieceAvailable { available, .. }) => {
                p.available = available;
            }
            (&mut Resource::Piece(ref mut p),
             SResourceUpdate::PieceDownloaded { downloaded, .. }) => {
                p.downloaded = downloaded;
            }
            (&mut Resource::Tracker(ref mut t),
             SResourceUpdate::TrackerStatus {
                 ref mut last_report,
                 ref mut error,
                 ..
             }) => {
                mem::swap(&mut t.last_report, last_report);
                mem::swap(&mut t.error, error);
            }
            (&mut Resource::File(ref mut f), SResourceUpdate::FilePriority { priority, .. }) => {
                f.priority = priority;
            }
            (&mut Resource::File(ref mut f), SResourceUpdate::FileProgress { progress, .. }) => {
                f.progress = progress;
            }
            _ => {}
        }
    }
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &Resource::Server(ref t) => {
                write!(f, "Server {{")?;
                write!(f, "\n")?;
                write!(f, "  id: {}", t.id)?;
                write!(f, "\n")?;
                write!(f, "  upload: {} B/s", t.rate_up)?;
                write!(f, "\n")?;
                write!(f, "  download: {} B/s", t.rate_down)?;
                write!(f, "\n")?;
                match t.throttle_up {
                    Some(u) if u >= 0 => {
                        write!(f, "  throttle up: {} B/s", u)?;
                    }
                    Some(u) => {
                        write!(f, "  throttle up: invalid({})", u)?;
                    }
                    None => {
                        write!(f, "  throttle up: unlimited")?;
                    }
                }
                write!(f, "\n")?;
                match t.throttle_down {
                    Some(u) if u >= 0 => {
                        write!(f, "  throttle down: {} B/s", u)?;
                    }
                    Some(u) => {
                        write!(f, "  throttle down: invalid({})", u)?;
                    }
                    None => {
                        write!(f, "  throttle down: unlimited")?;
                    }
                }
                write!(f, "\n")?;
                write!(f, "  uploaded: {} B", t.transferred_up)?;
                write!(f, "\n")?;
                write!(f, "  downloaded: {} B", t.transferred_down)?;
                write!(f, "\n")?;
                write!(f, "  session upload: {} B", t.ses_transferred_up)?;
                write!(f, "\n")?;
                write!(f, "  session download: {} B", t.ses_transferred_down)?;
                write!(f, "\n")?;
                write!(f, "  started at: {}", t.started)?;
                write!(f, "\n")?;
                write!(f, "}}")?;
            }
            &Resource::Torrent(ref t) => {
                write!(f, "Torrent {{")?;
                write!(f, "\n")?;
                write!(f, "  id: {}", t.id)?;
                write!(f, "\n")?;
                write!(
                    f,
                    "  name: {}",
                    if let Some(ref n) = t.name {
                        n.as_str()
                    } else {
                        "Unknown (magnet)"
                    }
                )?;
                write!(f, "\n")?;
                write!(f, "  path: {}", t.path)?;
                write!(f, "\n")?;
                write!(f, "  created at: {}", t.created)?;
                write!(f, "\n")?;
                write!(f, "  modified at: {}", t.modified)?;
                write!(f, "\n")?;
                write!(f, "  status: {}", t.status.as_str())?;
                write!(f, "\n")?;
                if let Some(ref e) = t.error {
                    write!(f, "  error: {}", e)?;
                    write!(f, "\n")?;
                }
                write!(f, "  priority: {}", t.priority)?;
                write!(f, "\n")?;
                write!(f, "  progress: {}", t.progress)?;
                write!(f, "\n")?;
                write!(f, "  availability: {}", t.availability)?;
                write!(f, "\n")?;
                write!(f, "  sequential: {}", t.sequential)?;
                write!(f, "\n")?;
                write!(f, "  upload: {} B/s", t.rate_up)?;
                write!(f, "\n")?;
                write!(f, "  download: {} B/s", t.rate_down)?;
                write!(f, "\n")?;
                match t.throttle_up {
                    Some(u) if u >= 0 => {
                        write!(f, "  throttle up: {} B/s", u)?;
                    }
                    Some(_) => {
                        write!(f, "  throttle up: unlimited")?;
                    }
                    None => {
                        write!(f, "  throttle up: server")?;
                    }
                }
                write!(f, "\n")?;
                match t.throttle_down {
                    Some(u) if u >= 0 => {
                        write!(f, "  throttle down: {} B/s", u)?;
                    }
                    Some(_) => {
                        write!(f, "  throttle down: unlimited")?;
                    }
                    None => {
                        write!(f, "  throttle down: server")?;
                    }
                }
                write!(f, "\n")?;
                write!(f, "  uploaded: {} B", t.transferred_up)?;
                write!(f, "\n")?;
                write!(f, "  downloaded: {} B", t.transferred_down)?;
                write!(f, "\n")?;
                write!(f, "  peers: {}", t.peers)?;
                write!(f, "\n")?;
                write!(f, "  trackers: {}", t.trackers)?;
                write!(f, "\n")?;
                if let Some(s) = t.size {
                    write!(f, "  size: {} B", s)?;
                } else {
                    write!(f, "  size: Unknown (magnet0")?;
                }
                write!(f, "\n")?;
                if let Some(p) = t.pieces {
                    write!(f, "  pieces: {}", p)?;
                } else {
                    write!(f, "  pieces: Unknown (magnet)")?;
                }
                write!(f, "\n")?;
                if let Some(p) = t.piece_size {
                    write!(f, "  piece size: {} B", p)?;
                } else {
                    write!(f, "  piece size: Unknown (magnet)")?;
                }
                write!(f, "\n")?;
                if let Some(files) = t.files {
                    write!(f, "  files: {}", files)?;
                } else {
                    write!(f, "  files: Unknown (magnet)")?;
                }
                write!(f, "\n")?;
                write!(f, "}}")?;
            }
            &Resource::File(ref t) => {
                write!(f, "{:#?}", t)?;
            }
            &Resource::Piece(ref t) => {
                write!(f, "{:#?}", t)?;
            }
            &Resource::Peer(ref t) => {
                write!(f, "{:#?}", t)?;
            }
            &Resource::Tracker(ref t) => {
                write!(f, "{:#?}", t)?;
            }
        }
        Ok(())
    }
}

// TODO: Consider how to handle datetime matching
// TODO: Proc macros to remove this shit

impl Filter for Resource {
    fn matches(&self, c: &Criterion) -> bool {
        match self {
            &Resource::Server(ref t) => t.matches(c),
            &Resource::Torrent(ref t) => t.matches(c),
            &Resource::File(ref t) => t.matches(c),
            &Resource::Piece(ref t) => t.matches(c),
            &Resource::Peer(ref t) => t.matches(c),
            &Resource::Tracker(ref t) => t.matches(c),
        }
    }
}

impl Filter for Server {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),

            "rate_up" => match_n(self.rate_up as i64, c),
            "rate_down" => match_n(self.rate_down as i64, c),
            "throttle_up" => {
                self.throttle_up.map(|v| match_n(v as i64, c)).unwrap_or(
                    false,
                )
            }
            "throttle_down" => {
                self.throttle_down.map(|v| match_n(v as i64, c)).unwrap_or(
                    false,
                )
            }

            _ => false,
        }
    }
}

impl Filter for Torrent {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),
            "name" => self.name.as_ref().map(|n| match_s(n, c)).unwrap_or(false),
            "path" => match_s(&self.path, c),
            "status" => match_s(self.status.as_str(), c),
            "error" => match_s(self.error.as_ref().map(|s| s.as_str()).unwrap_or(""), c),

            "priority" => match_n(self.priority as i64, c),
            "rate_up" => match_n(self.rate_up as i64, c),
            "rate_down" => match_n(self.rate_down as i64, c),
            "throttle_up" => self.throttle_up.map(|v| match_n(v, c)).unwrap_or(false),
            "throttle_down" => self.throttle_down.map(|v| match_n(v, c)).unwrap_or(false),
            "transferred_up" => match_n(self.transferred_up as i64, c),
            "transferred_down" => match_n(self.transferred_down as i64, c),
            "peers" => match_n(self.peers as i64, c),
            "trackers" => match_n(self.trackers as i64, c),
            "size" => self.size.map(|s| match_n(s as i64, c)).unwrap_or(false),
            "pieces" => self.pieces.map(|p| match_n(p as i64, c)).unwrap_or(false),
            "piece_size" => {
                self.piece_size.map(|p| match_n(p as i64, c)).unwrap_or(
                    false,
                )
            }
            "files" => self.files.map(|f| match_n(f as i64, c)).unwrap_or(false),

            "progress" => match_f(self.progress, c),
            "availability" => match_f(self.availability, c),

            "sequential" => match_b(self.sequential, c),

            _ => false,
        }
    }
}

impl Filter for Piece {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),
            "torrent_id" => match_s(&self.torrent_id, c),

            "available" => match_b(self.available, c),
            "downloaded" => match_b(self.downloaded, c),

            _ => false,
        }
    }
}

impl Filter for File {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),
            "torrent_id" => match_s(&self.torrent_id, c),
            "path" => match_s(&self.path, c),

            "priority" => match_n(self.priority as i64, c),

            "progress" => match_f(self.progress, c),

            _ => false,
        }
    }
}

impl Filter for Peer {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),
            "torrent_id" => match_s(&self.torrent_id, c),
            "ip" => match_s(&self.ip, c),

            "rate_up" => match_n(self.rate_up as i64, c),
            "rate_down" => match_n(self.rate_down as i64, c),

            "availability" => match_f(self.availability, c),

            // TODO: Come up with a way to match this
            "client_id" => false,

            _ => false,
        }
    }
}

impl Filter for Tracker {
    fn matches(&self, c: &Criterion) -> bool {
        match &c.field[..] {
            "id" => match_s(&self.id, c),
            "torrent_id" => match_s(&self.torrent_id, c),
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
