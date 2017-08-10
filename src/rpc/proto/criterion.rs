use regex::{self, Regex};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub field: String,
    pub op: Operation,
    pub value: Value,
}

#[derive(Copy, Clone, Debug, Deserialize, PartialEq)]
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

#[derive(Copy, Clone, Debug, Deserialize, PartialEq)]
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

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum Value {
    B(bool),
    S(String),
    N(u64),
    F(f32),
    AS(Vec<String>),
    AN(Vec<u64>),
    AF(Vec<f32>),
}

pub trait Filter {
    fn matches(&self, criterion: &Criterion) -> bool;
}

impl Default for ResourceKind {
    fn default() -> ResourceKind {
        ResourceKind::Torrent
    }
}

pub fn match_n<T: PartialOrd<u64>>(t: T, c: &Criterion) -> bool {
    match c.op {
        Operation::Eq => {
            match c.value {
                Value::N(v) => t.eq(&v),
                _ => false,
            }
        }
        Operation::Neq => {
            match c.value {
                Value::N(v) => t.ne(&v),
                _ => false,
            }
        }
        Operation::GT => {
            match c.value {
                Value::N(v) => t.gt(&v),
                _ => false,
            }
        }
        Operation::GTE => {
            match c.value {
                Value::N(v) => t.ge(&v),
                _ => false,
            }
        }
        Operation::LT => {
            match c.value {
                Value::N(v) => t.lt(&v),
                _ => false,
            }
        }
        Operation::LTE => {
            match c.value {
                Value::N(v) => t.le(&v),
                _ => false,
            }
        }
        Operation::In => {
            match c.value {
                Value::AN(ref a) => a.iter().any(|v| t.eq(v)),
                _ => false,
            }
        }
        Operation::NotIn => {
            match c.value {
                Value::AN(ref a) => a.iter().all(|v| t.ne(v)),
                _ => false,
            }
        }
        _ => false,
    }
}

pub fn match_f<T: PartialOrd<f32>>(t: T, c: &Criterion) -> bool {
    match c.op {
        Operation::Eq => {
            match c.value {
                Value::F(v) => t.eq(&v),
                _ => false,
            }
        }
        Operation::Neq => {
            match c.value {
                Value::F(v) => t.ne(&v),
                _ => false,
            }
        }
        Operation::GT => {
            match c.value {
                Value::F(v) => t.gt(&v),
                _ => false,
            }
        }
        Operation::GTE => {
            match c.value {
                Value::F(v) => t.ge(&v),
                _ => false,
            }
        }
        Operation::LT => {
            match c.value {
                Value::F(v) => t.lt(&v),
                _ => false,
            }
        }
        Operation::LTE => {
            match c.value {
                Value::F(v) => t.le(&v),
                _ => false,
            }
        }
        Operation::In => {
            match c.value {
                Value::AF(ref a) => a.iter().any(|v| t.eq(v)),
                _ => false,
            }
        }
        Operation::NotIn => {
            match c.value {
                Value::AF(ref a) => a.iter().all(|v| t.ne(v)),
                _ => false,
            }
        }
        _ => false,
    }
}

pub fn match_b(t: bool, c: &Criterion) -> bool {
    match c.op {
        Operation::Eq => {
            match c.value {
                Value::B(b) => t == b,
                _ => false,
            }
        }
        Operation::Neq => {
            match c.value {
                Value::B(b) => t != b,
                _ => false,
            }
        }
        _ => false,
    }
}

pub fn match_s(t: &str, c: &Criterion) -> bool {
    match c.op {
        Operation::Eq => {
            match c.value {
                Value::S(ref v) => v.eq(t),
                _ => false,
            }
        }
        Operation::Neq => {
            match c.value {
                Value::S(ref v) => v.ne(t),
                _ => false,
            }
        }
        Operation::Like => {
            match c.value {
                Value::S(ref v) => match_like(v, t),
                _ => false,
            }
        }
        Operation::ILike => {
            match c.value {
                Value::S(ref v) => match_ilike(v, t),
                _ => false,
            }
        }
        Operation::In => {
            match c.value {
                Value::AS(ref a) => a.iter().any(|v| t.eq(v)),
                _ => false,
            }
        }
        Operation::NotIn => {
            match c.value {
                Value::AS(ref a) => a.iter().all(|v| t.ne(v)),
                _ => false,
            }
        }
        _ => false,
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
