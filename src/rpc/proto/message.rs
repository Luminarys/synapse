use chrono::{DateTime, Utc};

use super::resource::{Resource, ResourceUpdate};
use super::criterion::Criterion;

/// Client -> server messages, deserialize only
#[derive(Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum CMessage {
    // Standard messages
    GetResources { serial: u64, ids: Vec<u64> },
    Subscribe { serial: u64, ids: Vec<u64> },
    Unsubscribe { serial: u64, ids: Vec<u64> },
    UpdateResource { serial: u64, resource: Resource },
    FilterSubscribe {
        serial: u64,
        criteria: Vec<Criterion>,
    },
    FilterUnsubscribe { serial: u64, filter_serial: u64 },

    // Special messages
    UploadTorrent { size: u64, path: Option<String> },
    UploadMagnet { uri: String, path: Option<String> },
    UploadFiles { size: u64, gzip: bool, path: String },
}

/// Server -> client message, serialize only
#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(tag = "type")]
pub enum SMessage {
    // Standard messages
    ResourcesExtant { serial: u64, ids: Vec<u64> },
    ResourcesRemoved { serial: u64, ids: Vec<u64> },
    UpdateResources {
        serial: u64,
        resources: Vec<ResourceUpdate>,
    },

    // Special messages
    TransferOffer {
        serial: u64,
        expires: DateTime<Utc>,
        token: String,
        size: u64,
    },

    // Error messages
    UnknownResource { reason: String },
    InvalidResource { reason: String },
    InvalidMessage { reason: String },
    InvalidSchema { reason: String },
    InvalidRequest { reason: String },
    PermissionDenied { reason: String },
    ServerError { reason: String },
}
