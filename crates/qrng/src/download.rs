//! Fetching a number of bytes from a source, one request at a time, no faster than it allows,
//! without blocking: the caller steps it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::fetch::{Fetch, FetchError, Secret, Ticket};
use crate::parse::{ParseError, parse_answer};
use crate::source::{Source, SourceError};

/// One answer's bytes in a download: where they start, how many, when they came and the pulse
/// the answer named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub offset: u64,
    pub bytes: u64,
    pub time: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse: Option<String>,
}

/// Why a download stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadError {
    /// The source sends a key and none is kept for it.
    NoKey,
    /// The source is disabled.
    Disabled,
    Source(SourceError),
    Fetch(FetchError),
    Parse(ParseError),
}

impl From<SourceError> for DownloadError {
    fn from(e: SourceError) -> Self {
        DownloadError::Source(e)
    }
}

impl fmt::Display for DownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DownloadError::NoKey => f.write_str(
                "the source needs a key and none is kept for it: add it in the window's source manager",
            ),
            DownloadError::Disabled => f.write_str("the source is disabled"),
            DownloadError::Source(e) => e.fmt(f),
            DownloadError::Fetch(e) => e.fmt(f),
            DownloadError::Parse(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for DownloadError {}

/// A download in progress.
#[derive(Debug)]
pub struct Download {
    source: Source,
    key: Option<Secret>,
    target: u64,
    data: Vec<u8>,
    chunks: Vec<Chunk>,
    in_flight: Option<Ticket>,
    last_start: Option<u64>,
    error: Option<DownloadError>,
    requests: u64,
}

impl Download {
    /// A download of at least `target` bytes from `source`, with its key if it sends one.
    pub fn new(source: Source, key: Option<Secret>, target: u64) -> Result<Self, DownloadError> {
        source.check()?;
        if !source.enabled {
            return Err(DownloadError::Disabled);
        }
        if source.key_header.is_some() && key.is_none() {
            return Err(DownloadError::NoKey);
        }
        Ok(Self {
            source,
            key,
            target,
            data: Vec::new(),
            chunks: Vec::new(),
            in_flight: None,
            last_start: None,
            error: None,
            requests: 0,
        })
    }

    /// Asks for at least `bytes` in all (never fewer than before).
    pub fn set_target(&mut self, bytes: u64) {
        self.target = self.target.max(bytes);
    }

    pub fn target(&self) -> u64 {
        self.target
    }

    /// Collects an answer that has arrived and starts the next request when more bytes are
    /// wanted and the source's spacing allows. Does nothing once failed.
    pub fn step(&mut self, fetch: &mut dyn Fetch) {
        if self.error.is_some() {
            return;
        }
        if let Some(t) = self.in_flight {
            match fetch.poll(t) {
                None => return,
                Some(Ok(body)) => match parse_answer(&self.source, &body) {
                    Ok((bytes, pulse)) => {
                        self.chunks.push(Chunk {
                            offset: self.data.len() as u64,
                            bytes: bytes.len() as u64,
                            time: crate::time::iso(fetch.now_ms()),
                            pulse,
                        });
                        self.data.extend(bytes);
                    }
                    Err(e) => self.error = Some(DownloadError::Parse(e)),
                },
                Some(Err(e)) => self.error = Some(DownloadError::Fetch(e)),
            }
            self.in_flight = None;
            if self.error.is_some() {
                return;
            }
        }
        let have = self.data.len() as u64;
        if have >= self.target {
            return;
        }
        let now = fetch.now_ms();
        if self
            .last_start
            .is_some_and(|l| now < l.saturating_add(self.source.min_interval_ms))
        {
            return;
        }
        match self.source.request(self.target - have) {
            Ok(request) => {
                self.in_flight = Some(fetch.start(&request, self.key.as_ref()));
                self.last_start = Some(now);
                self.requests += 1;
            }
            Err(e) => self.error = Some(e.into()),
        }
    }

    /// Has every byte asked for, and no request in flight.
    pub fn complete(&self) -> bool {
        self.error.is_none() && self.in_flight.is_none() && self.data.len() as u64 >= self.target
    }

    pub fn error(&self) -> Option<DownloadError> {
        self.error
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn requests(&self) -> u64 {
        self.requests
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    /// Milliseconds until the source allows the next request.
    pub fn wait_ms(&self, now_ms: u64) -> u64 {
        self.last_start.map_or(0, |l| {
            l.saturating_add(self.source.min_interval_ms)
                .saturating_sub(now_ms)
        })
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;
    use crate::source::presets;
    use crate::testing::ScriptedFetch;

    #[test]
    fn a_download_spaces_its_requests_and_keeps_every_byte() {
        let anu = presets().remove(0);
        let mut fetch = ScriptedFetch::anu(1);
        let mut d = Download::new(anu.clone(), Some(Secret::new("k")), 50_000).unwrap();
        let mut steps = 0;
        while !d.complete() && steps < 10_000 {
            d.step(&mut fetch);
            steps += 1;
        }
        assert!(d.complete(), "{:?}", d.error());
        // 50,000 bytes: two requests of 20,480 and one of 9,040 (452 values).
        assert_eq!(d.requests(), 3);
        assert_eq!(d.data().len(), 20_480 * 2 + 452 * 20);
        let starts = fetch.started_at();
        assert!(starts.windows(2).all(|w| w[1] - w[0] >= 1000), "{starts:?}");
        assert_eq!(d.chunks().len(), 3);
        assert!(fetch.keys_sent().iter().all(|k| k.as_deref() == Some("k")));
    }

    #[test]
    fn a_source_with_a_key_header_needs_a_key_and_a_failure_stops_it() {
        let anu = presets().remove(0);
        assert_eq!(
            Download::new(anu.clone(), None, 8).err(),
            Some(DownloadError::NoKey)
        );
        let mut fetch = ScriptedFetch::failing(FetchError::Status(403));
        let mut d = Download::new(anu, Some(Secret::new("k")), 8).unwrap();
        for _ in 0..10 {
            d.step(&mut fetch);
        }
        assert_eq!(
            d.error(),
            Some(DownloadError::Fetch(FetchError::Status(403)))
        );
        assert!(!d.complete());
    }
}
