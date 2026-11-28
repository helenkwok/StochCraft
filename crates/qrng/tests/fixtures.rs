//! The presets' parsers against answers recorded from the services (no test calls out).
//!
//! `fixtures/nist-beacon-pulse.json` is the NIST Randomness Beacon 2.0's latest pulse as served
//! by `https://beacon.nist.gov/beacon/2.0/pulse/last` on 2026-11-26 (pulse 2044989 of chain 2),
//! fetched once, unchanged.

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)] // A failed step fails the test.

use stochcraft_qrng::testing::ScriptedFetch;
use stochcraft_qrng::{Download, Fetch, parse::parse_answer, presets, seed};

const PULSE: &[u8] = include_bytes!("fixtures/nist-beacon-pulse.json");

fn beacon() -> stochcraft_qrng::Source {
    presets()
        .into_iter()
        .find(|p| p.id == "nist-beacon")
        .unwrap()
}

#[test]
fn a_beacon_pulse_gives_its_512_bit_output_and_its_uri() {
    let (bytes, pulse) = parse_answer(&beacon(), PULSE).unwrap();
    assert_eq!(bytes.len(), 64);
    let doc: serde_json::Value = serde_json::from_slice(PULSE).unwrap();
    let hex = doc["pulse"]["outputValue"]
        .as_str()
        .unwrap()
        .to_ascii_lowercase();
    assert_eq!(stochcraft_qrng::to_hex(&bytes), hex);
    assert_eq!(
        pulse.as_deref(),
        Some("https://beacon.nist.gov/beacon/2.0/chain/2/pulse/2044989")
    );
}

#[test]
fn a_seed_drawn_from_the_beacon_follows_the_rule() {
    let mut fetch = ScriptedFetch::always(PULSE.to_vec());
    let mut d = Download::new(beacon(), None, seed_bytes()).unwrap();
    while !d.complete() && d.error().is_none() {
        d.step(&mut fetch);
    }
    assert_eq!(d.requests(), 1);
    assert_eq!(fetch.keys_sent(), &[None]);
    let (bytes, _) = parse_answer(&beacon(), PULSE).unwrap();
    let s = seed::from_bytes(d.data());
    assert_eq!(s, seed::from_bytes(&bytes));
    assert!(s <= seed::MAX);
    assert!(
        d.chunks()[0]
            .pulse
            .as_deref()
            .is_some_and(|p| p.ends_with("/2044989"))
    );
    assert!(fetch.now_ms() > 0);
}

fn seed_bytes() -> u64 {
    stochcraft_qrng::estimate::SEED_BYTES
}
