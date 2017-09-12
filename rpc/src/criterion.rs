use regex::{self, Regex};
use chrono::{DateTime, Utc};

use resource::ResourceKind;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub field: String,
    pub op: Operation,
    pub value: Value,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq)]
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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum Value {
    B(bool),
    S(String),
    N(i64),
    F(f32),
    D(DateTime<Utc>),
    E(Option<()>),
    V(Vec<Value>),
}

pub enum Field<'a> {
    B(bool),
    S(&'a str),
    N(i64),
    F(f32),
    D(DateTime<Utc>),
    O(Box<Option<Field<'a>>>),
}

pub trait Queryable {
    fn field(&self, field: &str) -> Option<Field>;
}

impl Criterion {
    pub fn matches<Q: Queryable>(&self, q: &Q) -> bool {
        if let Some(f) = q.field(&self.field) {
            self.match_field(&f, self.op, &self.value)
        } else {
            false
        }
    }

    fn match_field(&self, f: &Field, op: Operation, value: &Value) -> bool {
        match (f, value) {
            (&Field::B(f), &Value::B(v)) => {
                match op {
                    Operation::Eq => f == v,
                    Operation::Neq => f != v,
                    _ => false,
                }
            }
            (&Field::S(ref f), &Value::S(ref v)) => {
                match op {
                    Operation::Eq => f == v,
                    Operation::Neq => f != v,
                    Operation::Like => match_like(f, v),
                    Operation::ILike => match_ilike(f, v),
                    _ => false,
                }
            }
            (&Field::N(f), &Value::N(v)) => {
                match op {
                    Operation::Eq => f == v,
                    Operation::Neq => f != v,
                    Operation::GTE => f >= v,
                    Operation::GT => f > v,
                    Operation::LTE => f <= v,
                    Operation::LT => f < v,
                    _ => false,
                }
            }
            (&Field::F(f), &Value::F(v)) => {
                match op {
                    Operation::Eq => f == v,
                    Operation::Neq => f != v,
                    Operation::GTE => f >= v,
                    Operation::GT => f > v,
                    Operation::LTE => f <= v,
                    Operation::LT => f < v,
                    _ => false,
                }
            }
            (&Field::D(f), &Value::D(v)) => {
                match op {
                    Operation::Eq => f == v,
                    Operation::Neq => f != v,
                    Operation::GTE => f >= v,
                    Operation::GT => f > v,
                    Operation::LTE => f <= v,
                    Operation::LT => f < v,
                    _ => false,
                }
            }
            (&Field::O(ref f), &Value::E(_)) => {
                match op {
                    Operation::Eq => f.is_none(),
                    Operation::Neq => f.is_some(),
                    _ => false,
                }
            }
            (&Field::O(ref b), v) => {
                match b.as_ref().as_ref() {
                    Some(f) => self.match_field(f, op, v),
                    None => false,
                }
            }
            (f, &Value::V(ref v)) => {
                match op {
                    Operation::In => v.iter().any(|item| self.match_field(f, op, item)),
                    Operation::NotIn => v.iter().all(|item| !self.match_field(f, op, item)),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

impl Default for ResourceKind {
    fn default() -> ResourceKind {
        ResourceKind::Torrent
    }
}

fn match_like(pat: &str, s: &str) -> bool {
    let mut p = regex::escape(pat);
    p = p.replace("%", ".*");
    p = p.replace("_", ".");
    if let Ok(re) = Regex::new(&p) {
        re.is_match(s)
    } else {
        false
    }
}

fn match_ilike(pat: &str, s: &str) -> bool {
    match_like(&pat.to_lowercase(), &s.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_like() {
        assert!(match_like("hello", "hello"));
        assert!(match_like("hello %", "hello world"));
        assert!(match_like("%world", "hello world"));
        assert!(!match_like("% world", "helloworld"));
        assert!(match_like("%", "foo bar"));
    }
}
