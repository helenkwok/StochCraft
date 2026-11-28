//! The parts of fetching in the browser (src/net.rs) that need no browser: an answer's body
//! gathered piece by piece under its cap, and the delay its timer is set for. Kept apart so they
//! can be tested natively.

/// An answer's body as its pieces arrive, never more than `max` bytes.
#[derive(Debug)]
pub struct Capped {
    max: u64,
    bytes: Vec<u8>,
}

impl Capped {
    pub fn new(max: u64) -> Self {
        Self {
            max,
            bytes: Vec::new(),
        }
    }

    /// Whether a piece of `len` more bytes stays within the cap (asked before it is copied).
    pub fn fits(&self, len: u64) -> bool {
        (self.bytes.len() as u64)
            .checked_add(len)
            .is_some_and(|n| n <= self.max)
    }

    /// Keeps `piece`, if it fits; `false` (and nothing kept) once the body would pass the cap,
    /// when the rest is not read.
    pub fn push(&mut self, piece: &[u8]) -> bool {
        if !self.fits(piece.len() as u64) {
            return false;
        }
        self.bytes.extend_from_slice(piece);
        true
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// The delay of the timer that stops a request after `timeout_ms`: what the browser's timer
/// takes, at most about 24.8 days (`i32::MAX` milliseconds).
pub fn delay_ms(timeout_ms: u64) -> i32 {
    i32::try_from(timeout_ms).unwrap_or(i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_is_kept_up_to_its_cap_and_refused_past_it() {
        let mut c = Capped::new(10);
        assert!(c.push(b"abcd"));
        assert!(c.push(b"efghij"));
        // Exactly at the cap is a valid answer; one byte more is not, and is not kept.
        assert!(!c.fits(1));
        assert!(!c.push(b"k"));
        assert_eq!(c.into_bytes(), b"abcdefghij");
        // A piece too large on its own is refused before it is copied.
        let c = Capped::new(4);
        assert!(!c.fits(u64::MAX));
        assert!(!c.fits(5));
        assert!(Capped::new(0).push(b""));
    }

    #[test]
    fn the_timer_takes_the_timeout_clamped_to_what_the_browser_allows() {
        assert_eq!(delay_ms(120_000), 120_000);
        assert_eq!(delay_ms(0), 0);
        assert_eq!(delay_ms(u64::MAX), i32::MAX);
    }
}
