use std::collections::BTreeMap;
use std::io::{Bytes, Cursor, Write, Read, self};

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
    ParseInt,
    EOF,
    IO,
}

pub fn encode<W: io::Write> (&ref b: &BEncode, w: &mut W) -> Result<(), io::Error> {
    // TODO: Make this either procedural or add recursion limit.
    match *b {
        BEncode::Int(i) => write!(w, "i{}e", i)?,
        BEncode::String(ref s) => {
            write!(w, "{}:", s.len())?;
            w.write(&s)?;
        },
        BEncode::List(ref v) => {
            write!(w, "l")?;
            for b in v.iter() {
                encode(b, w)?
            }
            write!(w, "e")?;
        }
        BEncode::Dict(ref d) => {
            write!(w, "d")?;
            for (k, v) in d.iter() {
                write!(w, "{}:{}", k.len(), k)?;
                encode(v, w)?;
            }
            write!(w, "e")?;
        }
    };
    Ok(())
}

fn decode_buf(bytes: &[u8]) -> Result<BEncode, BError> {
    return decode(&mut Cursor::new(bytes));
}

fn decode<R: io::Read>(bytes: &mut R) -> Result<BEncode, BError> {
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
        Ok(d) => {
            let mut slen = read_until(bytes, b':')?;
            slen.insert(0, d);
            let len = decode_int(slen)?;
            let mut v = vec![0u8; len as usize];
            bytes.read_exact(&mut v).map_err(|_| BError::EOF)?;
            Ok(BEncode::String(v))
        }
        Err(e) => Err(e)
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
fn test_encode() {
    let i = BEncode::Int(-10);
    let mut v = Vec::new();
    encode(&i, &mut v).unwrap();
    assert_eq!(v, b"i-10e");

    let s = BEncode::String(Vec::from(&b"asdf"[..]));
    v = Vec::new();
    encode(&s, &mut v).unwrap();
    assert_eq!(v, b"4:asdf");

    let l = BEncode::List(vec![i.clone(), s.clone()]);
    v = Vec::new();
    encode(&l, &mut v).unwrap();
    assert_eq!(v, b"li-10e4:asdfe");

    let mut map = BTreeMap::new();
    map.insert(String::from("asdf"), i.clone());
    let d = BEncode::Dict(map);
    v = Vec::new();
    encode(&d, &mut v).unwrap();
    assert_eq!(v, b"d4:asdfi-10ee");

    encode_decode(&i);
    encode_decode(&s);
    encode_decode(&l);
    encode_decode(&d);
}

fn encode_decode(b: &BEncode) {
    let mut v = Vec::new();
    encode(b, &mut v).unwrap();
    assert_eq!(b, &decode_buf(&v).unwrap());
}
