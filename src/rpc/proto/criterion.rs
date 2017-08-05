#[derive(Deserialize)]
pub struct Criterion {
    field: String,
    op: Operation,
    value: Value,
}

#[derive(Deserialize)]
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
pub enum Value {
    S(String),
    N(i64),
    F(f64),
    AS(Vec<String>),
    AN(Vec<i64>),
    AF(Vec<f64>),
}
