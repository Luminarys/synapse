#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    #[serde(default)]
    kind: ResourceKind,
    field: String,
    op: Operation,
    value: Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ResourceKind {
    Torrent,
    Peer,
    File,
    Piece,
    Tracker
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Operation {
    #[serde(rename = "==")]
    Eq,
    #[serde(rename = "!=")]
    Neq,
    #[serde(rename = ">")]
    GT,
    #[serde(rename = ">=")]
    GTE,
    #[serde(rename = "<")]
    LT,
    #[serde(rename = "<=")]
    LTE,
    #[serde(rename = "like")]
    Like,
    #[serde(rename = "ilike")]
    ILike,
    #[serde(rename = "in")]
    In,
    #[serde(rename = "!in")]
    NotIn,
}

#[derive(Deserialize)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum Value {
    S(String),
    N(i64),
    F(f64),
    AS(Vec<String>),
    AN(Vec<i64>),
    AF(Vec<f64>),
}

impl Default for ResourceKind {
    fn default() -> ResourceKind {
        ResourceKind::Torrent
    }
}
