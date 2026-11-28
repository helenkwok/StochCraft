//! A [`Fetch`] for tests, which never calls out: it answers from a script, after one poll, on a
//! clock that moves a quarter of a second each poll (and a tenth each time it is read), and
//! records what it was asked.

use std::cell::Cell;

use crate::fetch::{Fetch, FetchError, Secret, Ticket};
use crate::source::Request;

type Answer = Box<dyn FnMut(&Request) -> Result<Vec<u8>, FetchError> + Send>;

/// A scripted fetch.
pub struct ScriptedFetch {
    answer: Answer,
    now: Cell<u64>,
    next: u64,
    pending: Vec<(Ticket, Result<Vec<u8>, FetchError>, bool)>,
    started_at: Vec<u64>,
    urls: Vec<String>,
    keys: Vec<Option<String>>,
}

/// The time the scripted clock starts at: 2026-11-26T18:08:00Z.
pub const START_MS: u64 = 1_795_716_480_000;

/// The first `n` bytes of the stream [`ScriptedFetch::anu`] answers with.
pub fn anu_stream(seed: u64, n: usize) -> Vec<u8> {
    let mut r = stochcraft_random::Rng::new(seed);
    let mut out = Vec::with_capacity(n + 8);
    while out.len() < n {
        out.extend(r.next_u64().to_le_bytes());
    }
    out.truncate(n);
    out
}

impl ScriptedFetch {
    pub fn new(
        answer: impl FnMut(&Request) -> Result<Vec<u8>, FetchError> + Send + 'static,
    ) -> Self {
        Self {
            answer: Box::new(answer),
            now: Cell::new(START_MS),
            next: 1,
            pending: Vec::new(),
            started_at: Vec::new(),
            urls: Vec::new(),
            keys: Vec::new(),
        }
    }

    /// Answers as the ANU service does (`length` values of ten 16-bit hexadecimal blocks), the
    /// bytes taken in order from [`anu_stream`].
    pub fn anu(seed: u64) -> Self {
        let mut taken = 0usize;
        Self::new(move |r: &Request| {
            let length: usize = r
                .url
                .split(['?', '&'])
                .find_map(|p| p.strip_prefix("length="))
                .and_then(|n| n.parse().ok())
                .unwrap_or(1);
            let all = anu_stream(seed, taken + length * 20);
            let values: Vec<String> = all[taken..]
                .chunks(20)
                .map(crate::to_hex)
                .map(|h| format!("\"{h}\""))
                .collect();
            taken += length * 20;
            Ok(format!(
                "{{\"type\":\"hex16\",\"length\":{length},\"size\":10,\"data\":[{}],\"success\":true}}",
                values.join(",")
            )
            .into_bytes())
        })
    }

    /// Answers every request with `body`.
    pub fn always(body: Vec<u8>) -> Self {
        Self::new(move |_| Ok(body.clone()))
    }

    /// Fails every request with `e`.
    pub fn failing(e: FetchError) -> Self {
        Self::new(move |_| Err(e))
    }

    /// When each request started, by the scripted clock.
    pub fn started_at(&self) -> &[u64] {
        &self.started_at
    }

    /// The addresses requested.
    pub fn urls(&self) -> &[String] {
        &self.urls
    }

    /// The key each request carried.
    pub fn keys_sent(&self) -> &[Option<String>] {
        &self.keys
    }
}

impl Fetch for ScriptedFetch {
    fn start(&mut self, request: &Request, key: Option<&Secret>) -> Ticket {
        let t = Ticket(self.next);
        self.next += 1;
        self.started_at.push(self.now.get());
        self.urls.push(request.url.clone());
        self.keys.push(key.map(|k| k.expose().to_owned()));
        let answer = if key.is_some() && !request.url.starts_with("https://") {
            Err(FetchError::NotHttps)
        } else {
            (self.answer)(request)
        };
        self.pending.push((t, answer, false));
        t
    }

    fn poll(&mut self, ticket: Ticket) -> Option<Result<Vec<u8>, FetchError>> {
        self.now.set(self.now.get() + 250);
        let i = self.pending.iter().position(|p| p.0 == ticket)?;
        if !self.pending[i].2 {
            self.pending[i].2 = true;
            return None;
        }
        Some(self.pending.remove(i).1)
    }

    fn now_ms(&self) -> u64 {
        self.now.set(self.now.get() + 100);
        self.now.get()
    }
}
