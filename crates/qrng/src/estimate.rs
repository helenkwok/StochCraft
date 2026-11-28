//! What a draw, a file or a live run asks of a source: bytes, requests and time, against the
//! source's free tier where it has one.

use serde::Serialize;

use crate::source::Source;

/// The bytes a seed takes.
pub const SEED_BYTES: u64 = 8;

/// An estimate of what fetching some bytes from a source takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Estimate {
    pub bytes: u64,
    pub requests: u64,
    /// The least time it can take, from the source's spacing between requests (the answers
    /// themselves take longer).
    pub seconds_at_least: u64,
    /// The requests a month the source's free tier allows, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_requests_per_month: Option<u64>,
    /// The requests fit in a month of the free tier (when the source has one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub within_free_tier: Option<bool>,
}

/// What fetching `bytes` from `source` takes.
pub fn estimate(source: &Source, bytes: u64) -> Estimate {
    let requests = bytes.max(1).div_ceil(source.bytes_per_request());
    let seconds = requests
        .saturating_sub(1)
        .saturating_mul(source.min_interval_ms)
        .div_ceil(1000);
    Estimate {
        bytes,
        requests,
        seconds_at_least: seconds,
        free_requests_per_month: source.free_requests_per_month,
        within_free_tier: source.free_requests_per_month.map(|f| requests <= f),
    }
}

/// The bytes a run of `replications` reading `segment_bytes` each takes.
pub fn run_bytes(replications: u64, segment_bytes: u64) -> u64 {
    replications.saturating_mul(segment_bytes)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;
    use crate::source::presets;

    /// ANU: 20 bytes a value, 1024 values a request, one request a second, 100 free a month.
    #[test]
    fn anu_quota_maths() {
        let anu = &presets()[0];
        let seed = estimate(anu, SEED_BYTES);
        assert_eq!((seed.requests, seed.within_free_tier), (1, Some(true)));
        // 100 replications of 16 KiB: 1,638,400 bytes, 80 requests of 20,480 bytes.
        let live = estimate(anu, run_bytes(100, 16_384));
        assert_eq!(live.requests, 80);
        assert_eq!(live.seconds_at_least, 79);
        assert_eq!(live.within_free_tier, Some(true));
        let big = estimate(anu, run_bytes(1000, 65_536));
        assert_eq!(big.requests, 3200);
        assert_eq!(big.within_free_tier, Some(false));
        let beacon = estimate(&presets()[1], SEED_BYTES);
        assert_eq!((beacon.requests, beacon.free_requests_per_month), (1, None));
    }
}
