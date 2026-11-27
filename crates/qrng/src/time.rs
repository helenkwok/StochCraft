//! Times as records write them. The crate reads no clock: the app's [`crate::Fetch`] says what
//! time it is.

/// `ms` milliseconds after 1970-01-01T00:00:00Z, as ISO 8601 in UTC to the second
/// (`2026-11-27T01:02:03Z`).
pub fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

/// The year, month and day `days` days after 1970-01-01 (the proleptic Gregorian calendar).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    #[test]
    fn known_instants() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso(951_782_400_000), "2000-02-29T00:00:00Z");
        // The NIST beacon fixture's pulse time stamp.
        assert_eq!(super::iso(1_795_716_480_000), "2026-11-26T18:08:00Z");
    }
}
