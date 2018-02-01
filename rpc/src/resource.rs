use std::mem;
use std::fmt;
use std::borrow::Cow;

use chrono::{DateTime, Utc};
use serde;
use serde_json as json;
use url::Url;
use url_serde;

use super::criterion::{Field, Queryable};

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
    Resource(Cow<'a, Resource>),
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
    UserData {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        user_data: json::Value,
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

    PeerAvailability {
        id: String,
        #[serde(rename = "type")]
        kind: ResourceKind,
        availability: f32,
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
    #[serde(deserialize_with = "deserialize_throttle")]
    #[serde(default)]
    pub throttle_up: Option<Option<i64>>,
    #[serde(deserialize_with = "deserialize_throttle")]
    #[serde(default)]
    pub throttle_down: Option<Option<i64>>,
    pub user_data: Option<json::Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub id: String,
    pub download_token: String,
    pub rate_up: u64,
    pub rate_down: u64,
    pub throttle_up: Option<i64>,
    pub throttle_down: Option<i64>,
    pub transferred_up: u64,
    pub transferred_down: u64,
    pub ses_transferred_up: u64,
    pub ses_transferred_down: u64,
    pub started: DateTime<Utc>,
    pub user_data: json::Value,
}

impl Server {
    pub fn update(&mut self, update: &SResourceUpdate) {
        match update {
            &SResourceUpdate::Throttle {
                throttle_up,
                throttle_down,
                ..
            } => {
                self.throttle_up = throttle_up;
                self.throttle_down = throttle_down;
            }
            &SResourceUpdate::ServerTransfer {
                rate_up,
                rate_down,
                transferred_up,
                transferred_down,
                ses_transferred_up,
                ses_transferred_down,
                ..
            } => {
                self.rate_up = rate_up;
                self.rate_down = rate_down;
                self.transferred_up = transferred_up;
                self.transferred_down = transferred_down;
                self.ses_transferred_up = ses_transferred_up;
                self.ses_transferred_down = ses_transferred_down;
            }
            &SResourceUpdate::Rate {
                rate_up, rate_down, ..
            } => {
                self.rate_up = rate_up;
                self.rate_down = rate_down;
            }
            _ => {}
        }
    }
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
    pub user_data: json::Value,
}

impl Torrent {
    pub fn update(&mut self, update: &SResourceUpdate) {
        self.modified = Utc::now();
        match update {
            &SResourceUpdate::Throttle {
                throttle_up,
                throttle_down,
                ..
            } => {
                self.throttle_up = throttle_up;
                self.throttle_down = throttle_down;
            }
            &SResourceUpdate::TorrentStatus {
                ref error, status, ..
            } => {
                self.error = error.clone();
                self.status = status;
            }
            &SResourceUpdate::TorrentTransfer {
                rate_up,
                rate_down,
                transferred_up,
                transferred_down,
                progress,
                ..
            } => {
                self.rate_up = rate_up;
                self.rate_down = rate_down;
                self.transferred_up = transferred_up;
                self.transferred_down = transferred_down;
                self.progress = progress;
            }
            &SResourceUpdate::TorrentPeers {
                peers,
                availability,
                ..
            } => {
                self.peers = peers;
                self.availability = availability;
            }
            &SResourceUpdate::TorrentPicker { sequential, .. } => {
                self.sequential = sequential;
            }
            &SResourceUpdate::TorrentPriority { priority, .. } => {
                self.priority = priority;
            }
            _ => {}
        }
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[serde(deny_unknown_fields)]
pub enum Status {
    Pending,
    Magnet,
    Paused,
    Leeching,
    Idle,
    Seeding,
    Hashing,
    Error,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Piece {
    pub id: String,
    pub torrent_id: String,
    pub available: bool,
    pub downloaded: bool,
    pub index: u32,
    pub user_data: json::Value,
}

impl Piece {
    pub fn update(&mut self, update: &SResourceUpdate) {
        match update {
            &SResourceUpdate::PieceAvailable { available, .. } => {
                self.available = available;
            }
            &SResourceUpdate::PieceDownloaded { downloaded, .. } => {
                self.downloaded = downloaded;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub id: String,
    pub torrent_id: String,
    pub path: String,
    pub progress: f32,
    pub availability: f32,
    pub priority: u8,
    pub size: u64,
    pub user_data: json::Value,
}

impl File {
    pub fn update(&mut self, update: &SResourceUpdate) {
        match update {
            &SResourceUpdate::FilePriority { priority, .. } => {
                self.priority = priority;
            }
            &SResourceUpdate::FileProgress { progress, .. } => {
                self.progress = progress;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Peer {
    pub id: String,
    pub torrent_id: String,
    pub client_id: String,
    pub ip: String,
    pub rate_up: u64,
    pub rate_down: u64,
    pub availability: f32,
    pub user_data: json::Value,
}

impl Peer {
    pub fn update(&mut self, update: &SResourceUpdate) {
        match update {
            &SResourceUpdate::Rate {
                rate_up, rate_down, ..
            } => {
                self.rate_up = rate_up;
                self.rate_down = rate_down;
            }
            &SResourceUpdate::PeerAvailability { availability, .. } => {
                self.availability = availability;
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tracker {
    pub id: String,
    pub torrent_id: String,
    #[serde(with = "url_serde")]
    pub url: Option<Url>,
    pub last_report: DateTime<Utc>,
    pub error: Option<String>,
    pub user_data: json::Value,
}

impl Tracker {
    pub fn update(&mut self, update: &SResourceUpdate) {
        match update {
            &SResourceUpdate::TrackerStatus {
                ref last_report,
                ref error,
                ..
            } => {
                self.last_report = last_report.clone();
                self.error = error.clone();
            }
            _ => {}
        }
    }
}

impl<'a> SResourceUpdate<'a> {
    pub fn id(&self) -> &str {
        match self {
            &SResourceUpdate::Resource(ref r) => r.id(),
            &SResourceUpdate::Throttle { ref id, .. }
            | &SResourceUpdate::Rate { ref id, .. }
            | &SResourceUpdate::UserData { ref id, .. }
            | &SResourceUpdate::ServerTransfer { ref id, .. }
            | &SResourceUpdate::TorrentStatus { ref id, .. }
            | &SResourceUpdate::TorrentTransfer { ref id, .. }
            | &SResourceUpdate::TorrentPeers { ref id, .. }
            | &SResourceUpdate::TorrentPicker { ref id, .. }
            | &SResourceUpdate::TorrentPriority { ref id, .. }
            | &SResourceUpdate::TorrentPath { ref id, .. }
            | &SResourceUpdate::FilePriority { ref id, .. }
            | &SResourceUpdate::FileProgress { ref id, .. }
            | &SResourceUpdate::TrackerStatus { ref id, .. }
            | &SResourceUpdate::PeerAvailability { ref id, .. }
            | &SResourceUpdate::PieceAvailable { ref id, .. }
            | &SResourceUpdate::PieceDownloaded { ref id, .. } => id,
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

    pub fn user_data(&mut self) -> &mut json::Value {
        match self {
            &mut Resource::Server(ref mut r) => &mut r.user_data,
            &mut Resource::Torrent(ref mut r) => &mut r.user_data,
            &mut Resource::File(ref mut r) => &mut r.user_data,
            &mut Resource::Piece(ref mut r) => &mut r.user_data,
            &mut Resource::Peer(ref mut r) => &mut r.user_data,
            &mut Resource::Tracker(ref mut r) => &mut r.user_data,
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

    pub fn update(&mut self, update: &SResourceUpdate) {
        match self {
            &mut Resource::Server(ref mut s) => {
                s.update(update);
            }
            &mut Resource::Torrent(ref mut t) => {
                t.update(update);
            }
            &mut Resource::Piece(ref mut p) => {
                p.update(update);
            }
            &mut Resource::File(ref mut f) => {
                f.update(update);
            }
            &mut Resource::Peer(ref mut p) => {
                p.update(update);
            }
            &mut Resource::Tracker(ref mut t) => {
                t.update(update);
            }
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

fn deserialize_throttle<'de, D>(de: D) -> Result<Option<Option<i64>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let deser_result = serde::Deserialize::deserialize(de)?;
    match deser_result {
        json::Value::Null => Ok(Some(None)),
        json::Value::Number(ref i) if i.is_i64() => Ok(Some(Some(i.as_i64().unwrap()))),
        json::Value::Number(_) => Err(serde::de::Error::custom("Throttle must not be a float")),
        _ => Err(serde::de::Error::custom("Throttle must be number or null")),
    }
}

// TODO: Proc macros to remove this shit

impl Queryable for Resource {
    fn field(&self, f: &str) -> Option<Field> {
        match self {
            &Resource::Server(ref t) => t.field(f),
            &Resource::Torrent(ref t) => t.field(f),
            &Resource::File(ref t) => t.field(f),
            &Resource::Piece(ref t) => t.field(f),
            &Resource::Peer(ref t) => t.field(f),
            &Resource::Tracker(ref t) => t.field(f),
        }
    }
}

impl Queryable for json::Value {
    fn field(&self, f: &str) -> Option<Field> {
        match self.pointer(f) {
            Some(&json::Value::Null) => Some(Field::O(Box::new(None))),
            Some(&json::Value::Bool(b)) => Some(Field::B(b)),
            Some(&json::Value::Number(ref n)) => {
                if n.is_f64() {
                    Some(Field::F(n.as_f64().unwrap() as f32))
                } else {
                    Some(Field::N(n.as_i64().unwrap()))
                }
            }
            Some(&json::Value::String(ref s)) => Some(Field::S(s)),
            Some(&json::Value::Array(_)) => None,
            Some(&json::Value::Object(_)) => None,
            None => None,
        }
    }
}

impl Queryable for Server {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),

            "rate_up" => Some(Field::N(self.rate_up as i64)),
            "rate_down" => Some(Field::N(self.rate_down as i64)),
            "throttle_up" => Some(Field::O(Box::new(self.throttle_up.map(|v| Field::N(v))))),
            "throttle_down" => Some(Field::O(Box::new(self.throttle_down.map(|v| Field::N(v))))),
            "transferred_up" => Some(Field::N(self.transferred_up as i64)),
            "transferred_down" => Some(Field::N(self.transferred_down as i64)),
            "ses_transferred_up" => Some(Field::N(self.ses_transferred_up as i64)),
            "ses_transferred_down" => Some(Field::N(self.ses_transferred_down as i64)),

            "started" => Some(Field::D(self.started)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
        }
    }
}

impl Queryable for Torrent {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),
            "name" => Some(Field::O(Box::new(
                self.name.as_ref().map(|v| Field::S(v.as_str())),
            ))),
            "path" => Some(Field::S(&self.path)),
            "status" => Some(Field::S(self.status.as_str())),
            "error" => Some(Field::O(Box::new(
                self.error.as_ref().map(|v| Field::S(v.as_str())),
            ))),

            "priority" => Some(Field::N(self.priority as i64)),
            "rate_up" => Some(Field::N(self.rate_up as i64)),
            "rate_down" => Some(Field::N(self.rate_down as i64)),
            "throttle_up" => Some(Field::O(Box::new(self.throttle_up.map(|v| Field::N(v))))),
            "throttle_down" => Some(Field::O(Box::new(self.throttle_down.map(|v| Field::N(v))))),
            "transferred_up" => Some(Field::N(self.transferred_up as i64)),
            "transferred_down" => Some(Field::N(self.transferred_down as i64)),
            "peers" => Some(Field::N(self.peers as i64)),
            "trackers" => Some(Field::N(self.trackers as i64)),
            "size" => Some(Field::O(Box::new(self.size.map(|v| Field::N(v as i64))))),
            "pieces" => Some(Field::O(Box::new(self.pieces.map(|v| Field::N(v as i64))))),
            "piece_size" => Some(Field::O(Box::new(
                self.piece_size.map(|v| Field::N(v as i64)),
            ))),
            "files" => Some(Field::O(Box::new(self.files.map(|v| Field::N(v as i64))))),

            "created" => Some(Field::D(self.created)),
            "modified" => Some(Field::D(self.modified)),

            "progress" => Some(Field::F(self.progress)),
            "availability" => Some(Field::F(self.availability)),

            "sequential" => Some(Field::B(self.sequential)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
        }
    }
}

impl Queryable for Piece {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),
            "torrent_id" => Some(Field::S(&self.torrent_id)),

            "available" => Some(Field::B(self.available)),
            "downloaded" => Some(Field::B(self.downloaded)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
        }
    }
}

impl Queryable for File {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),
            "torrent_id" => Some(Field::S(&self.torrent_id)),
            "path" => Some(Field::S(&self.path)),

            "priority" => Some(Field::N(self.priority as i64)),

            "progress" => Some(Field::F(self.progress)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
        }
    }
}

impl Queryable for Peer {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),
            "torrent_id" => Some(Field::S(&self.torrent_id)),
            "ip" => Some(Field::S(&self.ip)),

            "rate_up" => Some(Field::N(self.rate_up as i64)),
            "rate_down" => Some(Field::N(self.rate_down as i64)),

            "availability" => Some(Field::F(self.availability)),

            "client_id" => Some(Field::S(&self.client_id)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
        }
    }
}

impl Queryable for Tracker {
    fn field(&self, f: &str) -> Option<Field> {
        match f {
            "id" => Some(Field::S(&self.id)),
            "torrent_id" => Some(Field::S(&self.torrent_id)),
            "url" => Some(Field::O(Box::new(
                self.url.as_ref().map(|u| Field::S(u.as_str())),
            ))),
            "error" => Some(Field::O(Box::new(
                self.error.as_ref().map(|v| Field::S(v.as_str())),
            ))),

            "last_report" => Some(Field::D(self.last_report)),

            _ if f.starts_with("user_data") => self.user_data.field(&f[9..]),

            _ => None,
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
            Status::Magnet => "magnet",
            Status::Error => "error",
        }
    }
}

/// Merges json objects according to RFC 7396
pub fn merge_json(original: &mut json::Value, update: &mut json::Value) {
    match (original, update) {
        (&mut json::Value::Object(ref mut o), &mut json::Value::Object(ref mut u)) => {
            for (k, v) in u.iter_mut() {
                if v.is_null() {
                    o.remove(k);
                } else if o.contains_key(k) {
                    merge_json(o.get_mut(k).unwrap(), v);
                } else {
                    o.insert(k.to_owned(), mem::replace(v, json::Value::Null));
                }
            }
        }
        (o, u) => {
            mem::swap(o, u);
        }
    }
}

impl Default for Status {
    fn default() -> Self {
        Status::Pending
    }
}

impl Default for Server {
    fn default() -> Self {
        Server {
            id: "".to_owned(),
            rate_up: 0,
            rate_down: 0,
            throttle_up: None,
            throttle_down: None,
            transferred_up: 0,
            transferred_down: 0,
            ses_transferred_up: 0,
            ses_transferred_down: 0,
            download_token: "".to_owned(),
            started: Utc::now(),
            user_data: json::Value::Null,
        }
    }
}

impl Default for Torrent {
    fn default() -> Self {
        Torrent {
            id: "".to_owned(),
            name: None,
            path: "".to_owned(),
            created: Utc::now(),
            modified: Utc::now(),
            status: Default::default(),
            error: None,
            priority: 0,
            progress: 0.,
            availability: 0.,
            sequential: false,
            rate_up: 0,
            rate_down: 0,
            throttle_up: None,
            throttle_down: None,
            transferred_up: 0,
            transferred_down: 0,
            peers: 0,
            trackers: 0,
            size: None,
            pieces: None,
            piece_size: None,
            files: None,
            user_data: json::Value::Null,
        }
    }
}

impl Default for Tracker {
    fn default() -> Self {
        Tracker {
            id: "".to_owned(),
            torrent_id: "".to_owned(),
            url: None,
            last_report: Utc::now(),
            error: None,
            user_data: json::Value::Null,
        }
    }
}
