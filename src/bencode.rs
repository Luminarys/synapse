use std::collections::BTreeMap;
use std::io;
use std::str;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BEncode {
    Int(i64),
    String(Vec<u8>),
    List(Vec<BEncode>),
    Dict(BTreeMap<String, BEncode>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BError {
    UTF8Decode,
    InvalidDict,
    InvalidChar,
    ParseInt,
    EOF,
    IO,
}

impl BEncode {
    pub fn to_int(self) -> Option<i64> {
        match self {
            BEncode::Int(v) => Some(v),
            _ => None,
        }
    }

    pub fn to_bytes(self) -> Option<Vec<u8>> {
        match self {
            BEncode::String(v) => Some(v),
            _ => None,
        }
    }

    pub fn to_string(self) -> Option<String> {
        match self {
            BEncode::String(v) => String::from_utf8(v).ok(),
            _ => None,
        }
    }

    pub fn to_list(self) -> Option<Vec<BEncode>> {
        match self {
            BEncode::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn to_dict(self) -> Option<BTreeMap<String, BEncode>> {
        match self {
            BEncode::Dict(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<&i64> {
        match *self {
            BEncode::Int(ref v) => Some(v),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&Vec<u8>> {
        match *self {
            BEncode::String(ref v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match *self {
            BEncode::String(ref v) => str::from_utf8(v).ok(),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&Vec<BEncode>> {
        match *self {
            BEncode::List(ref v) => Some(v),
            _ => None,
        }
    }

    pub fn as_dict(&self) -> Option<&BTreeMap<String, BEncode>> {
        match *self {
            BEncode::Dict(ref v) => Some(v),
            _ => None,
        }
    }

    pub fn encode<W: io::Write> (&self, w: &mut W) -> io::Result<()> {
        // TODO: Make this either procedural or add recursion limit.
        match *self {
            BEncode::Int(i) => write!(w, "i{}e", i)?,
            BEncode::String(ref s) => {
                write!(w, "{}:", s.len())?;
                w.write_all(&s)?;
            },
            BEncode::List(ref v) => {
                write!(w, "l")?;
                for b in v.iter() {
                    b.encode(w)?
                }
                write!(w, "e")?;
            }
            BEncode::Dict(ref d) => {
                write!(w, "d")?;
                for (k, v) in d.iter() {
                    write!(w, "{}:{}", k.len(), k)?;
                    v.encode(w)?;
                }
                write!(w, "e")?;
            }
        };
        Ok(())
    }
}

#[test]
pub fn decode_buf(bytes: &[u8]) -> Result<BEncode, BError> {
    return decode(&mut Cursor::new(bytes));
}

pub fn decode<R: io::Read>(bytes: &mut R) -> Result<BEncode, BError> {
    match next_byte(bytes) {
        Ok(b'i') => {
            let s = read_until(bytes, b'e')?;
            Ok(BEncode::Int(decode_int(s)?))
        }
        Ok(b'l') => {
            let mut l = vec![];
            loop {
                match decode(bytes) {
                    Ok(val) => l.push(val),
                    Err(BError::EOF) => break,
                    e @ Err(_) => return e,
                }
            }
            Ok(BEncode::List(l))
        }
        Ok(b'd') => {
            let mut d = BTreeMap::new();
            loop {
                let key = match decode(bytes) {
                    Ok(BEncode::String(s)) => String::from_utf8(s).map_err(|_| BError::UTF8Decode)?,
                    Ok(_) => return Err(BError::InvalidDict),
                    Err(BError::EOF) => break,
                    Err(e) => return Err(e),
                };
                d.insert(key, decode(bytes)?);
            };
            Ok(BEncode::Dict(d))
        }
        Err(BError::EOF) | Ok(b'e') => Err(BError::EOF),
        Ok(d @ b'0'...b'9') => {
            let mut slen = read_until(bytes, b':')?;
            slen.insert(0, d);
            let len = decode_int(slen)?;
            let mut v = vec![0u8; len as usize];
            bytes.read_exact(&mut v).map_err(|_| BError::EOF)?;
            Ok(BEncode::String(v))
        }
        Err(e) => Err(e),
        _ => Err(BError::InvalidChar),
    }
}

fn next_byte<R: io::Read>(r: &mut R) -> Result<u8, BError> {
    let mut v = [0];
    let amnt = r.read(&mut v).map_err(|_| BError::IO)?;
    if amnt == 0 {
        Err(BError::EOF)
    } else {
        Ok(v[0])
    }
}

fn read_until<R: io::Read>(r: &mut R, b: u8) -> Result<Vec<u8>, BError> {
    let mut v = vec![];
    loop {
        let n = next_byte(r)?;
        if b == n {
            return Ok(v);
        }
        v.push(n);
    }
}

fn decode_int(v: Vec<u8>) -> Result<i64, BError> {
    String::from_utf8(v).map_err(|_| BError::UTF8Decode).and_then(|i| {
        i.parse().map_err(|_| BError::ParseInt)
    })
}

#[test]
use std::io::Cursor;

#[test]
fn test_encode_decode() {
    let i = BEncode::Int(-10);
    let mut v = Vec::new();
    i.encode(&mut v).unwrap();
    assert_eq!(v, b"i-10e");

    let s = BEncode::String(Vec::from(&b"asdf"[..]));
    v = Vec::new();
    s.encode(&mut v).unwrap();
    assert_eq!(v, b"4:asdf");

    let s2r = [1u8,2,3,4];
    let s2e = [52u8, 58, 1, 2, 3, 4];
    let s2 = BEncode::String(Vec::from(&s2r[..]));
    v = Vec::new();
    s2.encode(&mut v).unwrap();
    assert_eq!(v, &s2e);

    let l = BEncode::List(vec![i.clone(), s.clone()]);
    v = Vec::new();
    l.encode(&mut v).unwrap();
    assert_eq!(v, b"li-10e4:asdfe");

    let mut map = BTreeMap::new();
    map.insert(String::from("asdf"), i.clone());
    map.insert(String::from("qwerty"), i.clone());
    let d = BEncode::Dict(map);
    v = Vec::new();
    d.encode(&mut v).unwrap();
    assert_eq!(v, b"d4:asdfi-10e6:qwertyi-10ee");

    decode_encode(b"d4:asdfi-10e6:qwertyi-10ee");

    encode_decode(&i);
    encode_decode(&s);
    encode_decode(&s2);
    encode_decode(&l);
    encode_decode(&d);
}

#[test]
fn encode_decode(b: &BEncode) {
    let mut v = Vec::new();
    b.encode(&mut v).unwrap();
    assert_eq!(b, &decode_buf(&v).unwrap());
}

#[test]
fn decode_encode(d: &[u8]) {
    let mut v = Vec::new();
    decode_buf(d).unwrap().encode(&mut v);
    assert_eq!(d, &v[..]);
}
