//! JSON-RPC framing over a byte stream: a `Content-Length` header, a blank
//! line, then the JSON body.

use std::io::{self, BufRead, Write};

use serde_json::Value;

/// Write one message.
pub fn write_message(w: &mut impl Write, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()
}

/// Read one message; `None` at the end of the stream. Headers other than
/// `Content-Length` are skipped; a body that is not JSON is an error.
pub fn read_message(r: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            // Stray blank lines before a header.
            continue;
        }
        if let Some((name, value)) = header.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().ok();
            }
        }
    }
    let length = length.unwrap_or(0);
    let mut body = vec![0; length];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn messages_round_trip_with_multibyte_bodies() {
        let mut buf = Vec::new();
        write_message(&mut buf, &json!({"a": "中文"})).unwrap();
        write_message(&mut buf, &json!({"b": 2})).unwrap();
        let text = String::from_utf8(buf.clone()).unwrap();
        assert!(text.starts_with("Content-Length: 14\r\n\r\n"), "{text}");
        let mut r = io::BufReader::new(&buf[..]);
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"a": "中文"})));
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({"b": 2})));
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn other_headers_are_skipped() {
        let raw = b"Content-Type: application/vscode-jsonrpc\r\nContent-Length: 2\r\n\r\n{}";
        let mut r = io::BufReader::new(&raw[..]);
        assert_eq!(read_message(&mut r).unwrap(), Some(json!({})));
    }
}
