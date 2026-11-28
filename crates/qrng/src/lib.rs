//! Where random bytes come from, apart from StochCraft's own generator: quantum random number
//! services, a public randomness beacon and services the user adds.
//!
//! This crate does no I/O. It describes the sources ([`Source`], [`presets`]), builds their
//! requests ([`Request`]), reads their answers ([`parse`]), estimates what a run asks of a source
//! ([`estimate`]), checks the bytes ([`health`]), turns a draw into a seed ([`seed`]), and gives
//! the format of a file of recorded bytes ([`record`]). Each app does the fetching through the
//! [`Fetch`] trait and keeps keys through [`KeyStore`]: a key is handed to a fetch and never
//! appears in a request, an error, a record or anything this crate writes.
//!
//! A [`Download`] drives a [`Fetch`] towards a number of bytes, one request at a time and no
//! faster than the source allows, without blocking: the caller steps it, between frames in the
//! browser.

mod download;
pub mod estimate;
mod fetch;
pub mod health;
pub mod parse;
pub mod record;
pub mod seed;
mod source;
pub mod testing;
pub mod time;

pub use download::{Chunk, Download, DownloadError};
pub use estimate::{Estimate, estimate};
pub use fetch::{Fetch, FetchError, KeyError, KeyStore, MemoryKeys, NoFetch, Secret, Ticket};
pub use health::{Health, check as health_check};
pub use parse::{ParseError, parse};
pub use source::{
    Encoding, Format, Kind, Method, PresetState, Request, Source, SourceChange, SourceError,
    Sources, SourcesSettings, presets,
};

/// `bytes` as lowercase hexadecimal.
pub fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

/// The SHA-256 of `bytes`, in hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    to_hex(&Sha256::digest(bytes))
}
