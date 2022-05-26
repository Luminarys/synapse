use url::percent_encoding::percent_encode_byte;

#[derive(Debug)]
pub struct RequestBuilder<'a> {
    method: &'a str,
    path: &'a str,
    base_query: Option<&'a str>,
    query_pairs: Vec<QueryPair<'a>>,
    headers: Vec<HttpHeader<'a>>,
}

#[derive(Debug)]
pub struct HttpHeader<'a> {
    name: &'a str,
    value: &'a str,
}

#[derive(Debug)]
pub struct QueryPair<'a> {
    name: &'a str,
    value: &'a [u8],
}

pub trait EncodeToBuf {
    fn encode(&self, buf: &mut Vec<u8>);
}

impl<'a> RequestBuilder<'a> {
    pub fn new(method: &'a str, path: &'a str, query: Option<&'a str>) -> RequestBuilder<'a> {
        RequestBuilder {
            method,
            path,
            base_query: query,
            query_pairs: Vec::with_capacity(10),
            headers: Vec::with_capacity(10),
        }
    }
}

impl<'a> RequestBuilder<'a> {
    pub fn query(&mut self, name: &'a str, value: &'a [u8]) -> &mut RequestBuilder<'a> {
        self.query_pairs.push(QueryPair { name, value });
        self
    }

    pub fn query_opt(&mut self, name: &'a str, value: Option<&'a [u8]>) -> &mut RequestBuilder<'a> {
        if let Some(v) = value {
            self.query_pairs.push(QueryPair { name, value: v });
        }
        self
    }

    pub fn header(&mut self, name: &'a str, value: &'a str) -> &mut RequestBuilder<'a> {
        self.headers.push(HttpHeader { name, value });
        self
    }
}

impl<'a> RequestBuilder<'a> {
    pub fn encode(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.method.as_bytes());
        buf.extend_from_slice(b" ");
        buf.extend_from_slice(self.path.as_bytes());

        buf.extend_from_slice(b"?");
        if let Some(q) = self.base_query {
            buf.extend_from_slice(q.as_bytes());
            buf.extend_from_slice(b"&");
        }
        for pair in &self.query_pairs {
            buf.extend_from_slice(pair.name.as_bytes());
            buf.extend_from_slice(b"=");
            encode_param(pair.value, buf);
            buf.extend_from_slice(b"&");
        }
        // The query either encodes an extra ? or an extra &, pop either off
        buf.pop();
        buf.extend_from_slice(b" HTTP/1.0");
        buf.extend_from_slice(b"\r\n");
        for header in &self.headers {
            buf.extend_from_slice(header.name.as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(header.value.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(b"\r\n");
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
        let mut encoded = Vec::new();
        RequestBuilder::new("GET", "/foobar/baz", Some("a=b"))
            .query("b", "c".as_bytes())
            .header("header1", "value1")
            .header("header2", "value2")
            .encode(&mut encoded);
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
        let mut encoded = Vec::new();
        RequestBuilder::new("GET", "/foobar/baz", Some("a=b"))
            .query_opt("a", None)
            .query_opt("b", Some(b"c"))
            .encode(&mut encoded);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            vec!["GET /foobar/baz?a=b&b=c HTTP/1.1", "\r\n",].join("\r\n")
        );
    }

    #[test]
    fn test_percent_encode_query() {
        let mut encoded = Vec::new();
        let req = RequestBuilder::new("GET", "/foobar/baz", None)
            .query("a", "&".as_bytes())
            .encode(&mut encoded);
        assert_eq!(
            String::from_utf8(encoded).unwrap(),
            vec!["GET /foobar/baz?a=%26 HTTP/1.1", "\r\n",].join("\r\n")
        );
    }
}
