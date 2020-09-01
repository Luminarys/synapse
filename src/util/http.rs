use url::percent_encoding::percent_encode_byte;

#[derive(Debug)]
pub struct RequestBuilder<'a, T, Q> {
    method: &'a str,
    path: &'a str,
    query: Q,
    headers: T,
}

#[derive(Debug)]
pub struct HttpHeader<'a, T> {
    name: &'a str,
    value: &'a str,
    headers: T,
}

#[derive(Debug)]
pub struct QueryPair<'a, T> {
    name: &'a str,
    value: &'a [u8],
    pairs: T,
}

#[derive(Debug)]
pub struct EmptyHeader {}

#[derive(Debug)]
pub struct BaseQuery<'a>(&'a str);

pub trait EncodeToBuf {
    fn encode(&self, buf: &mut Vec<u8>);
}

impl<'a> RequestBuilder<'a, EmptyHeader, BaseQuery<'a>> {
    pub fn new(
        method: &'a str,
        path: &'a str,
        query: Option<&'a str>,
    ) -> RequestBuilder<'a, EmptyHeader, BaseQuery<'a>> {
        RequestBuilder {
            method,
            path,
            query: BaseQuery(query.unwrap_or("")),
            headers: EmptyHeader {},
        }
    }
}

impl<'a, 'b, 'c, T, Q> RequestBuilder<'a, T, Q> {
    pub fn query(self, name: &'c str, value: &'c [u8]) -> RequestBuilder<'a, T, QueryPair<'c, Q>> {
        RequestBuilder {
            method: self.method,
            path: self.path,
            query: QueryPair {
                name,
                value,
                pairs: self.query,
            },
            headers: self.headers,
        }
    }

    pub fn query_opt(
        self,
        name: &'c str,
        value: Option<&'c [u8]>,
    ) -> RequestBuilder<'a, T, QueryPair<'c, Q>> {
        RequestBuilder {
            method: self.method,
            path: self.path,
            query: QueryPair {
                name: if value.is_some() { name } else { "" },
                value: value.unwrap_or(b""),
                pairs: self.query,
            },
            headers: self.headers,
        }
    }

    pub fn header(self, name: &'b str, value: &'b str) -> RequestBuilder<'a, HttpHeader<'b, T>, Q> {
        RequestBuilder {
            method: self.method,
            path: self.path,
            query: self.query,
            headers: HttpHeader {
                name,
                value,
                headers: self.headers,
            },
        }
    }
}

impl<'a, T: EncodeToBuf, Q: EncodeToBuf> RequestBuilder<'a, T, Q> {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.method.as_bytes());
        buf.extend_from_slice(b" ");
        buf.extend_from_slice(self.path.as_bytes());
        // The query either encodes an extra ? or an extra &, pop either off
        self.query.encode(buf);
        buf.pop();
        buf.extend_from_slice(b" HTTP/1.1");
        buf.extend_from_slice(b"\r\n");
        self.headers.encode(buf);
        buf.extend_from_slice(b"\r\n");
    }
}

impl<'a, T: EncodeToBuf> EncodeToBuf for HttpHeader<'a, T> {
    fn encode(&self, buf: &mut Vec<u8>) {
        self.headers.encode(buf);
        buf.extend_from_slice(self.name.as_bytes());
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(self.value.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
}

impl EncodeToBuf for EmptyHeader {
    fn encode(&self, _: &mut Vec<u8>) {}
}

impl<'a, T: EncodeToBuf> EncodeToBuf for QueryPair<'a, T> {
    fn encode(&self, buf: &mut Vec<u8>) {
        self.pairs.encode(buf);
        if self.name != "" {
            buf.extend_from_slice(self.name.as_bytes());
            buf.extend_from_slice(b"=");
            encode_param(self.value, buf);
            buf.extend_from_slice(b"&");
        }
    }
}

impl<'a> EncodeToBuf for BaseQuery<'a> {
    fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(b"?");
        buf.extend_from_slice(self.0.as_bytes());
        // Add an & if it the base query didn't contain it.
        if !self.0.is_empty() && !self.0.ends_with("&") {
            buf.extend_from_slice(b"&");
        }
    }
}

fn encode_param(param: &[u8], buf: &mut Vec<u8>) {
    for byte in param {
        let c = char::from(*byte);
        let mut char_buf = [0u8; 4];
        if (*byte > 0x20 && *byte < 0x7E) && (c.is_numeric() || c.is_alphabetic() || c == '-') {
            c.encode_utf8(&mut char_buf);
            buf.extend_from_slice(&char_buf[0..c.len_utf8()]);
        } else {
            buf.extend_from_slice(percent_encode_byte(*byte).as_bytes());
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_encode_http_request() {
        let req = RequestBuilder::new("GET", "/foobar/baz", Some("a=b"))
            .query("b", "c".as_bytes())
            .header("header1", "value1")
            .header("header2", "value2");
        let mut encoded = Vec::new();
        req.encode(&mut encoded);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            vec![
                "GET /foobar/baz?a=b&b=c HTTP/1.1",
                "header1: value1",
                "header2: value2",
                "\r\n",
            ]
            .join("\r\n")
        );
    }

    #[test]
    fn test_encode_opt_param() {
        let req = RequestBuilder::new("GET", "/foobar/baz", Some("a=b"))
            .query_opt("a", None)
            .query_opt("b", Some(b"c"));
        let mut encoded = Vec::new();
        req.encode(&mut encoded);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            vec!["GET /foobar/baz?a=b&b=c HTTP/1.1", "\r\n",].join("\r\n")
        );
    }

    #[test]
    fn test_percent_encode_query() {
        let req = RequestBuilder::new("GET", "/foobar/baz", None).query("a", "&".as_bytes());
        let mut encoded = Vec::new();
        req.encode(&mut encoded);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            vec!["GET /foobar/baz?a=%26 HTTP/1.1", "\r\n",].join("\r\n")
        );
    }
}
