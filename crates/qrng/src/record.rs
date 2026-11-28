//! The recorded-file format: the bytes fetched from a source, as a plain file of bytes in the
//! order they came (what a run reads with `random` `byte_file`), and beside it a manifest,
//! `<file>.json`, saying where they came from.
//!
//! The manifest gives the format's name ([`FORMAT`]), the source (id, name, kind and address
//! without its query), the bytes and their SHA-256, the requests made, each answer's bytes with
//! the time it came and the pulse it named, and the health check. It never holds a key.

use std::io::{self, Write};

use serde::{Deserialize, Serialize};

use crate::download::Chunk;
use crate::health::Health;
use crate::source::{Kind, Source};

/// The manifest's format.
pub const FORMAT: &str = "stochcraft-random-bytes/1";

/// Where a recorded file's manifest goes: beside it, `.json` added.
pub fn manifest_path(file: &str) -> String {
    format!("{file}.json")
}

/// A recorded file's manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub source: String,
    pub name: String,
    pub kind: Kind,
    /// The source's address without its query.
    pub url: String,
    pub bytes: u64,
    pub sha256: String,
    pub requests: u64,
    pub chunks: Vec<Chunk>,
    pub health: Health,
}

impl Manifest {
    /// The manifest of `data`, fetched from `source` in `chunks` with `requests` requests. The
    /// chunks are cut to `data`, so a manifest of the bytes recorded so far names no others.
    pub fn new(source: &Source, data: &[u8], chunks: &[Chunk], requests: u64) -> Self {
        let have = data.len() as u64;
        let chunks = chunks
            .iter()
            .filter(|c| c.offset < have)
            .map(|c| Chunk {
                bytes: c.bytes.min(have - c.offset),
                ..c.clone()
            })
            .collect();
        Self {
            format: FORMAT.to_owned(),
            source: source.id.clone(),
            name: source.name.clone(),
            kind: source.kind,
            url: source.shown_url(),
            bytes: data.len() as u64,
            sha256: crate::sha256_hex(data),
            requests,
            chunks,
            health: crate::health::check(data),
        }
    }

    /// As written to the file: pretty JSON with a final newline.
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).unwrap_or_default();
        s.push('\n');
        s
    }
}

/// Appends the bytes of `data` past `committed` to `out`, which holds the first `committed` so
/// far, counting each byte `out` takes: after a failure `committed` is the bytes in `out`, so a
/// retry writes only those still pending and a manifest of `data[..committed]` describes exactly
/// what was recorded.
pub fn append_pending(out: &mut impl Write, data: &[u8], committed: &mut usize) -> io::Result<()> {
    while let Some(rest) = data.get(*committed..).filter(|r| !r.is_empty()) {
        match out.write(rest) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => *committed += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    out.flush()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;
    use crate::source::presets;

    #[test]
    fn a_manifest_names_the_source_and_round_trips() {
        let anu = &presets()[0];
        let data = crate::testing::anu_stream(3, 4096);
        let chunks = [Chunk {
            offset: 0,
            bytes: 4096,
            time: "2026-11-27T00:00:00Z".into(),
            pulse: None,
        }];
        let m = Manifest::new(anu, &data, &chunks, 1);
        assert_eq!(m.url, "https://api.quantumnumbers.anu.edu.au");
        assert_eq!(m.sha256, crate::sha256_hex(&data));
        let back: Manifest = serde_json::from_str(&m.to_json()).unwrap();
        assert_eq!(back, m);
        assert_eq!(manifest_path("a/b.bin"), "a/b.bin.json");
    }

    /// Takes `room` bytes, then fails until given more room.
    struct Filling {
        out: Vec<u8>,
        room: usize,
    }

    impl Write for Filling {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if self.room == 0 {
                return Err(io::Error::other("disk full"));
            }
            let n = buf.len().min(self.room).min(7);
            self.room -= n;
            self.out.extend_from_slice(&buf[..n]);
            Ok(n)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_write_that_fails_partway_counts_what_landed_and_the_retry_writes_the_rest() {
        let anu = &presets()[0];
        let data = crate::testing::anu_stream(5, 100);
        let chunk = |offset, bytes| Chunk {
            offset,
            bytes,
            time: "2026-11-27T00:00:00Z".into(),
            pulse: None,
        };
        let chunks = [chunk(0, 40), chunk(40, 60)];
        let mut out = Filling {
            out: Vec::new(),
            room: 52,
        };
        let mut committed = 0;
        assert!(append_pending(&mut out, &data[..40], &mut committed).is_ok());
        assert_eq!(committed, 40);
        // The disk fills 12 bytes into the second answer.
        assert!(append_pending(&mut out, &data, &mut committed).is_err());
        assert_eq!(committed, 52);
        assert_eq!(out.out, data[..52]);
        // The manifest describes the bytes recorded, not those fetched.
        let m = Manifest::new(anu, &data[..committed], &chunks, 2);
        assert_eq!(m.bytes, 52);
        assert_eq!(m.sha256, crate::sha256_hex(&out.out));
        assert_eq!(m.chunks, [chunk(0, 40), chunk(40, 12)]);
        // The retry writes only what is pending, so nothing is written twice.
        out.room = 1000;
        assert!(append_pending(&mut out, &data, &mut committed).is_ok());
        assert_eq!(committed, 100);
        assert_eq!(out.out, data);
        assert_eq!(Manifest::new(anu, &data, &chunks, 2).chunks, chunks);
    }
}
