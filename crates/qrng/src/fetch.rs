//! What an app provides: fetching (without blocking) and a store for keys.

use std::collections::BTreeMap;
use std::fmt;

use crate::source::Request;

/// A key for a source. Its `Debug` and `Display` never show it.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key, to put in the request's header. Nothing else should call this.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(…)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("…")
    }
}

/// A request in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(pub u64);

/// Why a fetch failed: a fixed set of reasons, so no message from the network library, the
/// browser or the server (which could hold an address or a key) reaches a command's output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    /// The host could not be reached (no network, the name did not resolve, refused).
    Unreachable,
    Timeout,
    /// The secure connection failed (the certificate, for example).
    Tls,
    /// The service answered with this HTTP status.
    Status(u16),
    /// The service tried to send the request elsewhere: refused, so a key goes nowhere else.
    Redirected,
    /// The answer was larger than allowed.
    TooLarge,
    /// A key was to be sent other than over HTTPS: refused.
    NotHttps,
    /// The browser refused the request (another site's answers it does not allow, CORS).
    Blocked,
    /// This app cannot fetch.
    Unavailable,
    /// The fetch failed for another reason.
    Failed,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Unreachable => f.write_str("the service could not be reached"),
            FetchError::Timeout => f.write_str("the service did not answer in time"),
            FetchError::Tls => f.write_str("the secure connection to the service failed"),
            FetchError::Status(401 | 403) => f.write_str(
                "the service refused the request (HTTP 401 or 403): check the key",
            ),
            FetchError::Status(429) => f.write_str(
                "the service refused the request (HTTP 429): too many requests, or the quota is used up",
            ),
            FetchError::Status(s) => write!(f, "the service answered HTTP {s}"),
            FetchError::Redirected => f.write_str(
                "the service redirected the request elsewhere; it was not followed",
            ),
            FetchError::TooLarge => f.write_str("the answer was larger than allowed"),
            FetchError::NotHttps => f.write_str("a key is sent only over https://"),
            FetchError::Blocked => f.write_str(
                "the browser blocked the request (the service does not allow requests from other sites)",
            ),
            FetchError::Unavailable => f.write_str("this app cannot fetch from the network"),
            FetchError::Failed => f.write_str("the request failed"),
        }
    }
}

impl std::error::Error for FetchError {}

/// Fetching, without blocking: start a request, then poll it (between frames in the browser).
/// An implementation sends `key` in the request's `key_header` only over HTTPS, follows no
/// redirect, sends no cookies, caps the answer at `max_bytes`, and reports failures only as a
/// [`FetchError`].
pub trait Fetch: Send {
    fn start(&mut self, request: &Request, key: Option<&Secret>) -> Ticket;
    /// The answer's body once it has arrived (then the ticket is spent), `None` while waiting.
    fn poll(&mut self, ticket: Ticket) -> Option<Result<Vec<u8>, FetchError>>;
    /// The time now, in milliseconds since 1970 (UTC): for spacing requests and for records.
    fn now_ms(&self) -> u64;
}

/// Why a key could not be kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    /// There is no store (no keychain, no browser storage).
    Unavailable,
    /// The store refused it.
    Refused,
}

impl fmt::Display for KeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            KeyError::Unavailable => "there is no keychain or storage to keep the key in",
            KeyError::Refused => "the keychain refused to keep the key",
        })
    }
}

impl std::error::Error for KeyError {}

/// Where keys are kept: the OS keychain on the desktop, the browser's storage on the web. Keys
/// are kept under a source's `key_ref` and never written anywhere else.
pub trait KeyStore: Send {
    fn get(&self, key_ref: &str) -> Option<Secret>;
    fn set(&mut self, key_ref: &str, key: &Secret) -> Result<(), KeyError>;
    fn delete(&mut self, key_ref: &str);
    /// A warning about where keys are kept, for the window to show beside the key (the
    /// browser's storage, readable by anything that runs on the page).
    fn warning(&self) -> Option<&'static str> {
        None
    }
}

/// No fetching: every request fails with [`FetchError::Unavailable`].
#[derive(Debug, Default)]
pub struct NoFetch;

impl Fetch for NoFetch {
    fn start(&mut self, _: &Request, _: Option<&Secret>) -> Ticket {
        Ticket(0)
    }

    fn poll(&mut self, _: Ticket) -> Option<Result<Vec<u8>, FetchError>> {
        Some(Err(FetchError::Unavailable))
    }

    fn now_ms(&self) -> u64 {
        0
    }
}

/// Keys kept in memory for the session only (tests, and apps with no keychain).
#[derive(Debug, Default)]
pub struct MemoryKeys(BTreeMap<String, Secret>);

impl KeyStore for MemoryKeys {
    fn get(&self, key_ref: &str) -> Option<Secret> {
        self.0.get(key_ref).cloned()
    }

    fn set(&mut self, key_ref: &str, key: &Secret) -> Result<(), KeyError> {
        self.0.insert(key_ref.to_owned(), key.clone());
        Ok(())
    }

    fn delete(&mut self, key_ref: &str) {
        self.0.remove(key_ref);
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_prints() {
        let s = Secret::new("sk-live-123");
        assert!(!format!("{s:?} {s}").contains("sk-live"));
        assert_eq!(s.expose(), "sk-live-123");
        for e in [FetchError::Status(403), FetchError::Failed, FetchError::Tls] {
            assert!(!e.to_string().contains("sk-live"));
        }
    }
}
