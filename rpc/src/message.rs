use chrono::{DateTime, Utc};

use super::resource::{ResourceKind, CResourceUpdate, SResourceUpdate};
use super::criterion::Criterion;

#[derive(Debug, Serialize, Deserialize)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}

/// Client -> server messages
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum CMessage {
    // Standard messages
    GetResources { serial: u64, ids: Vec<String> },
    Subscribe { serial: u64, ids: Vec<String> },
    Unsubscribe { serial: u64, ids: Vec<String> },
    UpdateResource {
        serial: u64,
        resource: CResourceUpdate,
    },
    RemoveResource { serial: u64, id: String },
    FilterSubscribe {
        serial: u64,
        #[serde(default)]
        kind: ResourceKind,
        #[serde(default)]
        criteria: Vec<Criterion>,
    },
    FilterUnsubscribe { serial: u64, filter_serial: u64 },

    // Special messages
    UploadTorrent {
        serial: u64,
        size: u64,
        path: Option<String>,
        #[serde(default = "default_start")]
        start: bool,
    },
    UploadMagnet {
        serial: u64,
        uri: String,
        path: Option<String>,
        #[serde(default = "default_start")]
        start: bool,
    },
    UploadFiles {
        serial: u64,
        size: u64,
        path: String,
    },
    DownloadFile { serial: u64, id: String },
    PauseTorrent { serial: u64, id: String },
    ResumeTorrent { serial: u64, id: String },
    ValidateResources { serial: u64, ids: Vec<String> },
}

/// Server -> client message
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(tag = "type")]
#[serde(deny_unknown_fields)]
pub enum SMessage<'a> {
    // Standard messages
    #[serde(skip_deserializing)]
    ResourcesExtant { serial: u64, ids: Vec<&'a str> },
    #[serde(skip_serializing)]
    #[serde(rename = "RESOURCES_EXTANT")]
    OResourcesExtant { serial: u64, ids: Vec<String> },
    ResourcesRemoved { serial: u64, ids: Vec<String> },
    UpdateResources { resources: Vec<SResourceUpdate<'a>> },

    // Special messages
    RpcVersion(Version),
    TransferOffer {
        serial: u64,
        expires: DateTime<Utc>,
        token: String,
        size: u64,
    },

    // Error messages
    UnknownResource(Error),
    InvalidResource(Error),
    // InvalidMessage(Error),
    InvalidSchema(Error),
    // InvalidRequest(Error),
    PermissionDenied(Error),
    TransferFailed(Error),
    // ServerError(Error),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Error {
    pub serial: Option<u64>,
    pub reason: String,
}

impl Version {
    pub fn current() -> Version {
        Version {
            major: ::MAJOR_VERSION,
            minor: ::MINOR_VERSION,
        }
    }
}

fn default_start() -> bool {
    true
}

#[cfg(test)]
mod tests {
    extern crate serde_json;
    use super::*;
    use super::super::{resource, criterion};

    #[test]
    fn test_json_repr() {
        let data = r#"
            {
                "type": "FILTER_SUBSCRIBE",
                "serial": 0,
                "criteria": [
                    { "field": "id", "op": "in", "value": [1,2,3] }
                ]
            }
            "#;
        let m = serde_json::from_str(data).unwrap();
        if let CMessage::FilterSubscribe {
            kind: resource::ResourceKind::Torrent,
            serial: 0,
            criteria: c,
        } = m
        {
            assert_eq!(c[0].field, "id");
            assert_eq!(c[0].op, criterion::Operation::In);
            assert_eq!(c[0].value, criterion::Value::AN(vec![1, 2, 3]));
        } else {
            unreachable!();
        }
    }
}
