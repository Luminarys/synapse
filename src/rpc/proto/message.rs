use chrono::{DateTime, Utc};

use super::resource::{ResourceKind, CResourceUpdate, SResourceUpdate};
use super::criterion::Criterion;

/// Client -> server messages, deserialize only
#[derive(Debug, Deserialize)]
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
        criteria: Vec<Criterion>,
    },
    FilterUnsubscribe { serial: u64, filter_serial: u64 },

    // Special messages
    UploadTorrent {
        serial: u64,
        size: u64,
        path: Option<String>,
    },
    UploadMagnet {
        serial: u64,
        uri: String,
        path: Option<String>,
    },
    UploadFiles {
        serial: u64,
        size: u64,
        path: String,
    },
    DownloadFile { serial: u64, id: String },
}

/// Server -> client message, serialize only
#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(tag = "type")]
pub enum SMessage<'a> {
    // Standard messages
    ResourcesExtant { serial: u64, ids: Vec<&'a str> },
    ResourcesRemoved { serial: u64, ids: Vec<String> },
    UpdateResources { resources: Vec<SResourceUpdate<'a>> },

    // Special messages
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

#[derive(Serialize)]
pub struct Error {
    pub serial: Option<u64>,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::{resource, criterion};
    use serde_json;

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
