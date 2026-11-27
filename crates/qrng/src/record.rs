//! The recorded-file format: the bytes fetched from a source, as a plain file of bytes in the
//! order they came (what a run reads with `random` `byte_file`), and beside it a manifest,
//! `<file>.json`, saying where they came from.
//!
//! The manifest gives the format's name ([`FORMAT`]), the source (id, name, kind and address
//! without its query), the bytes and their SHA-256, the requests made, each answer's bytes with
//! the time it came and the pulse it named, and the health check. It never holds a key.

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
    /// The manifest of `data`, fetched from `source` in `chunks` with `requests` requests.
    pub fn new(source: &Source, data: &[u8], chunks: &[Chunk], requests: u64) -> Self {
        Self {
            format: FORMAT.to_owned(),
            source: source.id.clone(),
            name: source.name.clone(),
            kind: source.kind,
            url: source.shown_url(),
            bytes: data.len() as u64,
            sha256: crate::sha256_hex(data),
            requests,
            chunks: chunks.to_vec(),
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
}
