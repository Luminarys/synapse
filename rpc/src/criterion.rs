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

#[derive(Debug, Clone, PartialEq)]
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

    fn match_field(&self, field: &Field, op: Operation, value: &Value) -> bool {
        match (field, value) {
            (f, &Value::V(ref v)) => {
                match op {
                    Operation::In => {
                        v.iter().any(
                            |item| self.match_field(f, Operation::Eq, item),
                        )
                    }
                    Operation::NotIn => {
                        v.iter().all(
                            |item| self.match_field(f, Operation::Neq, item),
                        )
                    }
                    _ => false,
                }
            }
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
                    Operation::Like => match_like(v, f),
                    Operation::ILike => match_ilike(v, f),
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
                    None => {
                        match op {
                            Operation::Neq => true,
                            _ => false,
                        }
                    }
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
        assert!(match_like("fo%", "foo"));
    }

    struct Q;
    impl Queryable for Q {
        fn field(&self, f: &str) -> Option<Field> {
            match f {
                "s" => Some(Field::S("foo")),
                "n" => Some(Field::N(1)),
                "ob" => Some(Field::O(Box::new(Some(Field::B(true))))),
                "on" => Some(Field::O(Box::new(None))),
                _ => None,
            }
        }
    }

    #[test]
    fn test_match_bad_field() {
        let c = Criterion {
            field: "asdf".to_owned(),
            op: Operation::Like,
            value: Value::S("fo%".to_owned()),
        };

        let q = Q;
        assert_eq!(q.field("asdf"), None);
        assert!(!c.matches(&q));
    }

    #[test]
    fn test_match() {
        let c = Criterion {
            field: "s".to_owned(),
            op: Operation::Like,
            value: Value::S("fo%".to_owned()),
        };

        let q = Q;
        assert!(c.matches(&q));
    }

    #[test]
    fn test_match_none() {
        let c = Criterion {
            field: "on".to_owned(),
            op: Operation::Eq,
            value: Value::E(None),
        };

        let q = Q;
        assert!(c.matches(&q));
    }

    #[test]
    fn test_match_some() {
        let c = Criterion {
            field: "ob".to_owned(),
            op: Operation::Eq,
            value: Value::B(true),
        };

        let q = Q;
        assert!(c.matches(&q));
    }

    #[test]
    fn test_match_none_in() {
        let c = Criterion {
            field: "on".to_owned(),
            op: Operation::In,
            value: Value::V(vec![Value::B(false), Value::E(None)]),
        };

        let q = Q;
        assert!(c.matches(&q));
    }

    #[test]
    fn test_match_none_not_in() {
        let c = Criterion {
            field: "on".to_owned(),
            op: Operation::NotIn,
            value: Value::V(vec![Value::B(false), Value::B(true)]),
        };

        let q = Q;
        assert!(c.matches(&q));
    }
}
