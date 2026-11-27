//! Reading a source's answer: the random bytes in it, and the pulse it names.

use std::fmt;

use serde_json::Value;

use crate::source::{Encoding, Format, Source};

/// Why an answer could not be read. The answer's own text is never repeated: it could hold
/// anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    NotJson,
    /// The JSON path leads nowhere.
    NoPath,
    BadHex,
    BadBase64,
    BadNumber,
    /// A value of another type than the format expects.
    WrongType,
    /// The answer holds no bytes.
    Empty,
    /// The answer says the request failed (`"success": false`).
    Refused,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::NotJson => "the answer is not JSON",
            ParseError::NoPath => "the answer has nothing at the source's JSON path",
            ParseError::BadHex => "the answer is not hexadecimal where it should be",
            ParseError::BadBase64 => "the answer is not Base64 where it should be",
            ParseError::BadNumber => {
                "the answer has something other than whole numbers from 0 to 255"
            }
            ParseError::WrongType => "the answer has another kind of value at the JSON path",
            ParseError::Empty => "the answer holds no random bytes",
            ParseError::Refused => "the service answered that the request failed",
        })
    }
}

impl std::error::Error for ParseError {}

/// The random bytes in `body`, an answer of `format`.
pub fn parse(format: &Format, body: &[u8]) -> Result<Vec<u8>, ParseError> {
    let bytes = match format {
        Format::Raw => body.to_vec(),
        Format::Hex => hex(text(body)?)?,
        Format::Base64 => base64(text(body)?)?,
        Format::Numbers => numbers(text(body)?)?,
        Format::Json { path, encoding } => {
            let doc = json(body)?;
            from_value(at(&doc, path).ok_or(ParseError::NoPath)?, *encoding)?
        }
    };
    if bytes.is_empty() {
        return Err(ParseError::Empty);
    }
    Ok(bytes)
}

/// The random bytes in an answer from `source`, and the pulse it names (an `https://` address
/// at the source's pulse path, when it has one).
pub fn parse_answer(source: &Source, body: &[u8]) -> Result<(Vec<u8>, Option<String>), ParseError> {
    let bytes = parse(&source.format, body)?;
    let pulse = match &source.pulse_path {
        Some(p) => json(body)
            .ok()
            .and_then(|doc| at(&doc, p).and_then(Value::as_str).map(str::to_owned))
            .filter(|u| {
                u.starts_with("https://")
                    && u.len() <= 200
                    && u.bytes().all(|b| b.is_ascii_graphic())
            }),
        None => None,
    };
    Ok((bytes, pulse))
}

fn text(body: &[u8]) -> Result<&str, ParseError> {
    std::str::from_utf8(body).map_err(|_| ParseError::WrongType)
}

fn json(body: &[u8]) -> Result<Value, ParseError> {
    let doc: Value = serde_json::from_slice(body).map_err(|_| ParseError::NotJson)?;
    if doc.get("success") == Some(&Value::Bool(false)) {
        return Err(ParseError::Refused);
    }
    Ok(doc)
}

/// The value at `path`: keys and array indices separated by dots.
fn at<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(doc, |v, part| match v {
        Value::Array(items) => items.get(part.parse::<usize>().ok()?),
        _ => v.get(part),
    })
}

fn from_value(v: &Value, encoding: Encoding) -> Result<Vec<u8>, ParseError> {
    match (v, encoding) {
        (Value::Array(items), _) => {
            let mut out = Vec::new();
            for item in items {
                if item.is_array() {
                    return Err(ParseError::WrongType);
                }
                out.extend(from_value(item, encoding)?);
            }
            Ok(out)
        }
        (Value::String(s), Encoding::Hex) => hex(s),
        (Value::String(s), Encoding::Base64) => base64(s),
        (Value::String(s), Encoding::Numbers) => numbers(s),
        (Value::Number(n), Encoding::Numbers) => n
            .as_u64()
            .and_then(|n| u8::try_from(n).ok())
            .map(|b| vec![b])
            .ok_or(ParseError::BadNumber),
        _ => Err(ParseError::WrongType),
    }
}

/// The bytes of hexadecimal `s`, white space ignored.
pub fn hex(s: &str) -> Result<Vec<u8>, ParseError> {
    let digits: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|c| match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(ParseError::BadHex),
        })
        .collect::<Result<_, _>>()?;
    if !digits.len().is_multiple_of(2) {
        return Err(ParseError::BadHex);
    }
    Ok(digits.chunks(2).map(|p| (p[0] << 4) | p[1]).collect())
}

/// The bytes of Base64 `s` (standard or URL-safe alphabet, padded or not), white space ignored.
pub fn base64(s: &str) -> Result<Vec<u8>, ParseError> {
    let value = |c: u8| -> Result<u32, ParseError> {
        Ok(u32::from(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return Err(ParseError::BadBase64),
        }))
    };
    let chars: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let body: &[u8] = match chars.iter().position(|&c| c == b'=') {
        Some(p) => {
            if chars[p..].iter().any(|&c| c != b'=') || chars.len() - p > 2 {
                return Err(ParseError::BadBase64);
            }
            &chars[..p]
        }
        None => &chars,
    };
    if body.len() % 4 == 1 {
        return Err(ParseError::BadBase64);
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for group in body.chunks(4) {
        let mut n = 0u32;
        for &c in group {
            n = (n << 6) | value(c)?;
        }
        n <<= 6 * (4 - group.len() as u32);
        let three = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&three[..group.len() - 1]);
    }
    Ok(out)
}

/// The bytes of `s`: whole numbers from 0 to 255 separated by white space or commas.
pub fn numbers(s: &str) -> Result<Vec<u8>, ParseError> {
    s.split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<u8>().map_err(|_| ParseError::BadNumber))
        .collect()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;
    use crate::source::presets;

    #[test]
    fn hex_base64_and_numbers_read_the_same_bytes() {
        let want = vec![0u8, 1, 0x7f, 0x80, 0xfe, 0xff];
        assert_eq!(hex("00017F80 feff"), Ok(want.clone()));
        assert_eq!(base64("AAF/gP7/"), Ok(want.clone()));
        assert_eq!(base64("AAF_gP7_"), Ok(want.clone()));
        assert_eq!(numbers("0 1, 127\n128 254 255"), Ok(want.clone()));
        assert_eq!(base64("AAE="), Ok(vec![0, 1]));
        assert_eq!(base64("AAE"), Ok(vec![0, 1]));
        assert_eq!(base64("AA=E"), Err(ParseError::BadBase64));
        assert_eq!(hex("abc"), Err(ParseError::BadHex));
        assert_eq!(numbers("256"), Err(ParseError::BadNumber));
    }

    /// The shape of an ANU Quantum Numbers answer (from its documentation's example client:
    /// `success`, and `data`, an array of `length` strings of `size` 16-bit hexadecimal blocks).
    #[test]
    fn an_anu_answer_gives_twenty_bytes_a_value() {
        let anu = &presets()[0];
        let body = br#"{"type":"hex16","length":2,"size":10,"data":["00010203040506070809101112131415161718ff","a0a1a2a3a4a5a6a7a8a9b0b1b2b3b4b5b6b7b8b9"],"success":true}"#;
        let (bytes, pulse) = parse_answer(anu, body).unwrap();
        assert_eq!(bytes.len(), 40);
        assert_eq!(bytes[19], 0xff);
        assert_eq!(pulse, None);
        let refused = br#"{"success":false,"message":"Limit Exceeded"}"#;
        assert_eq!(parse_answer(anu, refused), Err(ParseError::Refused));
    }

    #[test]
    fn json_paths_reach_into_objects_and_arrays() {
        let f = Format::Json {
            path: "a.1.b".into(),
            encoding: Encoding::Numbers,
        };
        assert_eq!(parse(&f, br#"{"a":[0,{"b":[1,2,3]}]}"#), Ok(vec![1, 2, 3]));
        assert_eq!(parse(&f, br#"{"a":[]}"#), Err(ParseError::NoPath));
        assert_eq!(parse(&f, b"not json"), Err(ParseError::NotJson));
        assert_eq!(parse(&Format::Raw, b""), Err(ParseError::Empty));
    }

    #[test]
    fn random_org_plain_integers_are_bytes() {
        let org = &presets()[2];
        assert_eq!(
            parse_answer(org, b"12\n0\n255\n").map(|x| x.0),
            Ok(vec![12, 0, 255])
        );
    }

    #[test]
    fn lfd_answers_with_qrn_hex() {
        let lfd = presets().into_iter().find(|p| p.id == "lfd").unwrap();
        assert_eq!(
            parse_answer(&lfd, br#"{"qrn":"0aff","length":2}"#).map(|x| x.0),
            Ok(vec![10, 255])
        );
    }
}
