//! Sources: the presets, sources the user adds, and the requests they make.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The HTTP method a source is called with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Method {
    #[default]
    Get,
    Post,
}

/// How the random bytes are written inside an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Encoding {
    /// Hexadecimal digits, two per byte.
    Hex,
    /// Base64 (standard or URL-safe, padded or not).
    Base64,
    /// Whole numbers from 0 to 255, one per byte.
    Numbers,
}

/// The shape of a source's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Format {
    /// A JSON document; `path` (keys and array indices separated by dots, such as `data` or
    /// `pulse.outputValue`) leads to a string, a number or an array of them, written in
    /// `encoding`. An answer with `"success": false` at its top is a refusal.
    Json { path: String, encoding: Encoding },
    /// The whole answer is hexadecimal (white space ignored).
    Hex,
    /// The whole answer is Base64 (white space ignored).
    Base64,
    /// The whole answer is whole numbers from 0 to 255, separated by white space or commas.
    Numbers,
    /// The answer's bytes are the random bytes.
    Raw,
}

/// What kind of randomness a source gives, as the window labels it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A quantum random number generator.
    Quantum,
    /// A public randomness beacon: published for everyone, so never for secrets.
    Beacon,
    /// A physical source that is not quantum (atmospheric noise, for example).
    NotQuantum,
    /// Not known: a source the user added.
    #[default]
    Unknown,
}

fn one() -> u64 {
    1
}

fn yes() -> bool {
    true
}

/// A source of random bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Source {
    /// Its name in commands and settings: lowercase letters, digits and hyphens.
    pub id: String,
    /// Its name as the window shows it.
    pub name: String,
    /// The address, with `{n}` where the number of units to fetch goes. Over HTTPS whenever a
    /// key is sent; never with a key in it.
    pub url: String,
    #[serde(default)]
    pub method: Method,
    /// The header the key goes in (`x-api-key`); `None` for a source that needs no key.
    #[serde(default)]
    pub key_header: Option<String>,
    pub format: Format,
    /// The bytes one unit (`{n}` counts units) brings.
    #[serde(default = "one")]
    pub bytes_per_unit: u64,
    /// The most units one request may ask for.
    #[serde(default = "one")]
    pub max_units: u64,
    /// The least time between two requests, in milliseconds.
    #[serde(default)]
    pub min_interval_ms: u64,
    /// Serves seeds only: a few bytes at a time, not files or live runs.
    #[serde(default)]
    pub seed_only: bool,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Where its key is kept (the OS keychain, or the browser's storage): a name, never the
    /// key. Bound to the address's origin: a new origin clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<String>,
    /// A JSON path to the answer's own address (a beacon pulse's URI), recorded as provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pulse_path: Option<String>,
    /// The requests a month its free tier allows, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_requests_per_month: Option<u64>,
    /// A preset of StochCraft's (its address and format cannot be changed).
    #[serde(default)]
    pub preset: bool,
    /// Not listed, and cannot be enabled (a preset waiting for its operators' permission).
    #[serde(default)]
    pub hidden: bool,
    /// The browser cannot reach it (it allows no requests from other sites).
    #[serde(default)]
    pub desktop_only: bool,
}

/// A request to a source: everything but the key, which the [`crate::Fetch`] adds in the
/// source's `key_header`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub method: Method,
    pub url: String,
    /// The header the key goes in; the fetch refuses to send one other than over HTTPS, and
    /// follows no redirect.
    pub key_header: Option<String>,
    /// The most bytes the answer may have.
    pub max_bytes: u64,
    /// How long to wait for the answer, in milliseconds.
    pub timeout_ms: u64,
}

/// Why a source or a change to one is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    BadId,
    BadUrl,
    /// The address has a user name or password, or a query parameter that looks like a key.
    KeyInUrl,
    /// A key is sent, but not over HTTPS.
    KeyNeedsHttps,
    /// Asks for more than one unit, but the address has no `{n}`.
    NoCount,
    BadHeader,
    BadNumbers,
    BadPath,
    NotFound,
    /// A preset's address and format are its own.
    PresetFixed,
    /// The preset is waiting for its operators' permission.
    NotYetAllowed,
    /// A preset cannot be removed (it can be disabled).
    PresetKept,
    Exists,
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SourceError::BadId => {
                "a source's id is 1 to 40 lowercase letters, digits and hyphens"
            }
            SourceError::BadUrl => {
                "the address must start with https:// (or http:// for a source with no key) and name a host"
            }
            SourceError::KeyInUrl => {
                "the address holds what looks like a key (a user name, a password, or a query parameter named like key, token or secret): give the key in the window's source manager, where it is kept in the keychain, and its header name here"
            }
            SourceError::KeyNeedsHttps => "a source that sends a key must use https://",
            SourceError::NoCount => {
                "the address needs {n} where the number to fetch goes, unless a request brings one unit (max_units 1)"
            }
            SourceError::BadHeader => {
                "the key header is a header name: letters, digits and hyphens, not Host, Cookie or another reserved header"
            }
            SourceError::BadNumbers => {
                "bytes_per_unit and max_units must be at least 1 (and a request at most 16 MiB), min_interval_ms at most a day"
            }
            SourceError::BadPath => "the JSON path must be keys or indices separated by dots",
            SourceError::NotFound => "no source with that id",
            SourceError::PresetFixed => {
                "a preset's address, format and limits are its own: only `enabled` can be changed (add a source of your own for another)"
            }
            SourceError::NotYetAllowed => {
                "this source is not offered yet: it waits for its operators' permission for bulk use"
            }
            SourceError::PresetKept => "a preset cannot be removed; disable it instead",
            SourceError::Exists => "a preset has that id",
        })
    }
}

impl std::error::Error for SourceError {}

/// The most bytes one request may bring.
const MOST_PER_REQUEST: u64 = 16 << 20;

/// The parts of an `http(s)://` address: scheme, authority (host and port) and the rest.
fn split_url(url: &str) -> Option<(&str, &str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme, "https" | "http") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return None;
    }
    Some((scheme, authority, tail))
}

/// Query parameter names that look like they carry a key.
fn key_like(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "key",
        "token",
        "secret",
        "password",
        "passwd",
        "auth",
        "sig",
        "credential",
    ]
    .iter()
    .any(|w| n.contains(w))
}

/// A header name the key may go in: a token, and not one a client sets itself.
fn good_header(name: &str) -> bool {
    let reserved = [
        "host",
        "cookie",
        "content-length",
        "transfer-encoding",
        "connection",
        "user-agent",
        "origin",
        "referer",
    ];
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        && !reserved.contains(&name.to_ascii_lowercase().as_str())
}

fn good_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('.').all(|p| {
            !p.is_empty()
                && p.len() <= 64
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

impl Source {
    /// Checks the source: its id, its address (no key in it, HTTPS when a key is sent), its
    /// count placeholder, its header and its numbers.
    pub fn check(&self) -> Result<(), SourceError> {
        let id_ok = !self.id.is_empty()
            && self.id.len() <= 40
            && self
                .id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            return Err(SourceError::BadId);
        }
        let (scheme, authority, tail) = split_url(&self.url).ok_or(SourceError::BadUrl)?;
        if authority.contains('@') {
            return Err(SourceError::KeyInUrl);
        }
        if let Some((_, query)) = tail.split_once('?') {
            let query = query.split('#').next().unwrap_or("");
            if query
                .split('&')
                .any(|pair| key_like(pair.split('=').next().unwrap_or("")))
            {
                return Err(SourceError::KeyInUrl);
            }
        }
        if let Some(h) = &self.key_header {
            if !good_header(h) {
                return Err(SourceError::BadHeader);
            }
            if scheme != "https" {
                return Err(SourceError::KeyNeedsHttps);
            }
        }
        if self.bytes_per_unit == 0
            || self.max_units == 0
            || self.bytes_per_unit.saturating_mul(self.max_units) > MOST_PER_REQUEST
            || self.min_interval_ms > 86_400_000
        {
            return Err(SourceError::BadNumbers);
        }
        if self.max_units > 1 && !self.url.contains("{n}") {
            return Err(SourceError::NoCount);
        }
        if let Format::Json { path, .. } = &self.format
            && !good_path(path)
        {
            return Err(SourceError::BadPath);
        }
        if let Some(p) = &self.pulse_path
            && !good_path(p)
        {
            return Err(SourceError::BadPath);
        }
        Ok(())
    }

    /// `scheme://host[:port]` of its address.
    pub fn origin(&self) -> Option<String> {
        let (scheme, authority, _) = split_url(&self.url)?;
        Some(format!("{scheme}://{}", authority.to_ascii_lowercase()))
    }

    /// Its address as records and messages give it: the origin and path, without the query.
    pub fn shown_url(&self) -> String {
        match split_url(&self.url) {
            Some((scheme, authority, tail)) => {
                let path = tail.split(['?', '#']).next().unwrap_or("");
                format!("{scheme}://{}{path}", authority.to_ascii_lowercase())
            }
            None => String::new(),
        }
    }

    /// The name its key is kept under: its id and its address's origin, so a key is never sent
    /// to another origin than the one it was given for.
    pub fn expected_key_ref(&self) -> Option<String> {
        Some(format!("stochcraft-random {} {}", self.id, self.origin()?))
    }

    /// It sends a key, and has one bound to its current origin.
    pub fn key_bound(&self) -> bool {
        self.key_header.is_some()
            && self.key_ref.is_some()
            && self.key_ref == self.expected_key_ref()
    }

    /// The bytes one request brings at most.
    pub fn bytes_per_request(&self) -> u64 {
        self.bytes_per_unit.saturating_mul(self.max_units).max(1)
    }

    /// The request for `bytes` more bytes, at most one request's worth: the units that cover
    /// them, `{n}` filled in.
    pub fn request(&self, bytes: u64) -> Result<Request, SourceError> {
        self.check()?;
        let units = bytes.div_ceil(self.bytes_per_unit).clamp(1, self.max_units);
        let url = self.url.replace("{n}", &units.to_string());
        let wanted = units.saturating_mul(self.bytes_per_unit);
        Ok(Request {
            method: self.method,
            url,
            key_header: self.key_header.clone(),
            // Hexadecimal or numbers in JSON take several characters a byte.
            max_bytes: wanted.saturating_mul(8).saturating_add(64 << 10),
            timeout_ms: 120_000,
        })
    }
}

/// StochCraft's presets.
///
/// - `anu`: the ANU Quantum Numbers service (needs the user's own key; the free tier allows 100
///   requests a month, one a second): 1024 values of 10 hexadecimal 16-bit blocks (20 bytes) a
///   request.
/// - `nist-beacon`: the NIST Randomness Beacon 2.0's latest pulse, 512 bits a minute, signed and
///   chained; for seeds only, and public, so never for secrets.
/// - `random-org`: random.org's integer generator, 0 to 255; off until enabled, and not quantum
///   (atmospheric noise).
/// - `lfd`: the LfD QRNG of OTH Regensburg, hidden and disabled until its operators allow bulk
///   use; it allows no requests from other sites, so not in the browser.
pub fn presets() -> Vec<Source> {
    vec![
        Source {
            id: "anu".into(),
            name: "ANU Quantum Numbers".into(),
            url: "https://api.quantumnumbers.anu.edu.au?length={n}&type=hex16&size=10".into(),
            method: Method::Get,
            key_header: Some("x-api-key".into()),
            format: Format::Json {
                path: "data".into(),
                encoding: Encoding::Hex,
            },
            bytes_per_unit: 20,
            max_units: 1024,
            min_interval_ms: 1000,
            seed_only: false,
            kind: Kind::Quantum,
            enabled: true,
            key_ref: None,
            pulse_path: None,
            free_requests_per_month: Some(100),
            preset: true,
            hidden: false,
            desktop_only: false,
        },
        Source {
            id: "nist-beacon".into(),
            name: "NIST Randomness Beacon".into(),
            url: "https://beacon.nist.gov/beacon/2.0/pulse/last".into(),
            method: Method::Get,
            key_header: None,
            format: Format::Json {
                path: "pulse.outputValue".into(),
                encoding: Encoding::Hex,
            },
            bytes_per_unit: 64,
            max_units: 1,
            min_interval_ms: 60_000,
            seed_only: true,
            kind: Kind::Beacon,
            enabled: true,
            key_ref: None,
            pulse_path: Some("pulse.uri".into()),
            free_requests_per_month: None,
            preset: true,
            hidden: false,
            desktop_only: false,
        },
        Source {
            id: "random-org".into(),
            name: "random.org".into(),
            url: "https://www.random.org/integers/?num={n}&min=0&max=255&col=1&base=10&format=plain&rnd=new".into(),
            method: Method::Get,
            key_header: None,
            format: Format::Numbers,
            bytes_per_unit: 1,
            max_units: 10_000,
            min_interval_ms: 1000,
            seed_only: false,
            kind: Kind::NotQuantum,
            enabled: false,
            key_ref: None,
            pulse_path: None,
            free_requests_per_month: None,
            preset: true,
            hidden: false,
            desktop_only: false,
        },
        Source {
            id: "lfd".into(),
            name: "LfD QRNG (OTH Regensburg)".into(),
            url: "https://lfdr.de/qrng_api/qrng?length={n}&format=HEX".into(),
            method: Method::Get,
            key_header: None,
            format: Format::Json {
                path: "qrn".into(),
                encoding: Encoding::Hex,
            },
            bytes_per_unit: 1,
            max_units: 65_536,
            min_interval_ms: 1000,
            seed_only: false,
            kind: Kind::Quantum,
            enabled: false,
            key_ref: None,
            pulse_path: None,
            free_requests_per_month: None,
            preset: true,
            hidden: true,
            desktop_only: true,
        },
    ]
}

/// A change to the sources (`random.source.set`): a preset's `enabled`, or a source of the
/// user's own, added or replaced whole. Never a key.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceChange {
    pub id: String,
    /// Remove the user's source with this id.
    #[serde(default)]
    pub remove: bool,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    /// The address, with `{n}` for the count; never with a key in it.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub method: Option<Method>,
    /// The header name the key goes in (the key itself is entered in the window only).
    #[serde(default)]
    pub key_header: Option<String>,
    #[serde(default)]
    pub format: Option<Format>,
    #[serde(default)]
    pub bytes_per_unit: Option<u64>,
    #[serde(default)]
    pub max_units: Option<u64>,
    #[serde(default)]
    pub min_interval_ms: Option<u64>,
    #[serde(default)]
    pub seed_only: Option<bool>,
}

/// A preset's settings: whether it is on, and where its key is kept.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetState {
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<String>,
}

/// The sources as an app's settings keep them: the user's own sources and the presets' states.
/// No key is ever in it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourcesSettings {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub user: Vec<Source>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub presets: BTreeMap<String, PresetState>,
}

/// The sources a session knows: the presets, then the user's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sources {
    list: Vec<Source>,
}

impl Default for Sources {
    fn default() -> Self {
        Self { list: presets() }
    }
}

impl Sources {
    /// The presets, with `settings` applied; a user source that fails its check is left out.
    pub fn from_settings(settings: &SourcesSettings) -> Self {
        let mut s = Self::default();
        for p in &mut s.list {
            if let Some(state) = settings.presets.get(&p.id) {
                if let Some(on) = state.enabled
                    && !p.hidden
                {
                    p.enabled = on;
                }
                p.key_ref = state.key_ref.clone();
            }
        }
        for u in &settings.user {
            let mut u = u.clone();
            u.preset = false;
            u.hidden = false;
            u.kind = Kind::Unknown;
            u.pulse_path = None;
            u.free_requests_per_month = None;
            if u.check().is_ok() && s.get(&u.id).is_none() {
                s.list.push(u);
            }
        }
        s
    }

    /// What an app's settings keep of them.
    pub fn settings(&self) -> SourcesSettings {
        let defaults = presets();
        let mut out = SourcesSettings::default();
        for s in &self.list {
            if s.preset {
                let default_on = defaults.iter().find(|d| d.id == s.id).map(|d| d.enabled);
                let state = PresetState {
                    enabled: (Some(s.enabled) != default_on).then_some(s.enabled),
                    key_ref: s.key_ref.clone(),
                };
                if state != PresetState::default() {
                    out.presets.insert(s.id.clone(), state);
                }
            } else {
                out.user.push(s.clone());
            }
        }
        out
    }

    pub fn get(&self, id: &str) -> Option<&Source> {
        self.list.iter().find(|s| s.id == id)
    }

    /// Every source but the hidden ones.
    pub fn listed(&self) -> impl Iterator<Item = &Source> {
        self.list.iter().filter(|s| !s.hidden)
    }

    /// Applies `change`. Returns the source as it now is (`None` once removed) and the key
    /// reference that no longer applies, if any, for the caller to delete from its key store: a
    /// new address origin (or a source removed) unbinds its key.
    pub fn set(
        &mut self,
        change: &SourceChange,
    ) -> Result<(Option<Source>, Option<String>), SourceError> {
        let existing = self.list.iter().position(|s| s.id == change.id);
        if change.remove {
            let i = existing.ok_or(SourceError::NotFound)?;
            if self.list[i].preset {
                return Err(SourceError::PresetKept);
            }
            let gone = self.list.remove(i);
            return Ok((None, gone.key_ref));
        }
        if let Some(i) = existing
            && self.list[i].preset
        {
            let fixed = change.name.is_some()
                || change.url.is_some()
                || change.method.is_some()
                || change.key_header.is_some()
                || change.format.is_some()
                || change.bytes_per_unit.is_some()
                || change.max_units.is_some()
                || change.min_interval_ms.is_some()
                || change.seed_only.is_some();
            if fixed {
                return Err(SourceError::PresetFixed);
            }
            let p = &mut self.list[i];
            if let Some(on) = change.enabled {
                if p.hidden && on {
                    return Err(SourceError::NotYetAllowed);
                }
                p.enabled = on;
            }
            return Ok((Some(p.clone()), None));
        }
        let base = existing.map(|i| self.list[i].clone());
        let url = change
            .url
            .clone()
            .or_else(|| base.as_ref().map(|b| b.url.clone()))
            .ok_or(SourceError::BadUrl)?;
        let format = change
            .format
            .clone()
            .or_else(|| base.as_ref().map(|b| b.format.clone()))
            .unwrap_or(Format::Raw);
        let next = Source {
            id: change.id.clone(),
            name: change
                .name
                .clone()
                .or_else(|| base.as_ref().map(|b| b.name.clone()))
                .unwrap_or_else(|| change.id.clone()),
            url,
            method: change
                .method
                .or_else(|| base.as_ref().map(|b| b.method))
                .unwrap_or_default(),
            key_header: match &change.key_header {
                Some(h) if h.trim().is_empty() => None,
                Some(h) => Some(h.trim().to_owned()),
                None => base.as_ref().and_then(|b| b.key_header.clone()),
            },
            format,
            bytes_per_unit: change
                .bytes_per_unit
                .or_else(|| base.as_ref().map(|b| b.bytes_per_unit))
                .unwrap_or(1),
            max_units: change
                .max_units
                .or_else(|| base.as_ref().map(|b| b.max_units))
                .unwrap_or(1),
            min_interval_ms: change
                .min_interval_ms
                .or_else(|| base.as_ref().map(|b| b.min_interval_ms))
                .unwrap_or(1000),
            seed_only: change
                .seed_only
                .or_else(|| base.as_ref().map(|b| b.seed_only))
                .unwrap_or(false),
            kind: Kind::Unknown,
            enabled: change
                .enabled
                .or_else(|| base.as_ref().map(|b| b.enabled))
                .unwrap_or(true),
            key_ref: base.as_ref().and_then(|b| b.key_ref.clone()),
            pulse_path: None,
            free_requests_per_month: None,
            preset: false,
            hidden: false,
            desktop_only: false,
        };
        next.check()?;
        let mut next = next;
        // The key was given for the old origin (or the source sends none now): unbind it.
        let mut dropped = None;
        if next.key_ref.is_some() && (next.key_header.is_none() || !next.key_bound()) {
            dropped = next.key_ref.take();
        }
        match existing {
            Some(i) => self.list[i] = next.clone(),
            None => self.list.push(next.clone()),
        }
        Ok((Some(next), dropped))
    }

    /// Binds a key to source `id` at its current origin: the reference to keep it under.
    pub fn bind_key(&mut self, id: &str) -> Result<String, SourceError> {
        let s = self
            .list
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or(SourceError::NotFound)?;
        if s.key_header.is_none() {
            return Err(SourceError::BadHeader);
        }
        let r = s.expected_key_ref().ok_or(SourceError::BadUrl)?;
        s.key_ref = Some(r.clone());
        Ok(r)
    }

    /// Forgets source `id`'s key binding; the reference it had, to delete from the key store.
    pub fn unbind_key(&mut self, id: &str) -> Option<String> {
        self.list
            .iter_mut()
            .find(|s| s.id == id)
            .and_then(|s| s.key_ref.take())
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)] // A failed step fails the test.
mod tests {
    use super::*;

    fn user(url: &str) -> SourceChange {
        SourceChange {
            id: "mine".into(),
            url: Some(url.into()),
            format: Some(Format::Hex),
            max_units: Some(64),
            ..SourceChange::default()
        }
    }

    #[test]
    fn the_presets_pass_their_own_checks() {
        for p in presets() {
            assert_eq!(p.check(), Ok(()), "{}", p.id);
        }
    }

    #[test]
    fn the_anu_request_asks_for_hex16_blocks_of_ten() {
        let anu = presets().remove(0);
        let r = anu.request(20 * 1024 * 3).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            r.url,
            "https://api.quantumnumbers.anu.edu.au?length=1024&type=hex16&size=10"
        );
        assert_eq!(r.key_header.as_deref(), Some("x-api-key"));
        let small = anu.request(8).unwrap_or_else(|e| panic!("{e}"));
        assert!(small.url.contains("length=1&"), "{}", small.url);
        assert_eq!(anu.shown_url(), "https://api.quantumnumbers.anu.edu.au");
    }

    #[test]
    fn a_key_in_the_address_is_refused() {
        let mut s = Sources::default();
        for url in [
            "https://x.example/r?n={n}&api_key=abc",
            "https://x.example/r?token=abc&n={n}",
            "https://user:pw@x.example/r?n={n}",
        ] {
            assert_eq!(s.set(&user(url)), Err(SourceError::KeyInUrl), "{url}");
        }
    }

    #[test]
    fn a_key_needs_https_and_a_count_needs_its_placeholder() {
        let mut s = Sources::default();
        let mut c = user("http://x.example/r?n={n}");
        c.key_header = Some("x-key".into());
        assert_eq!(s.set(&c), Err(SourceError::KeyNeedsHttps));
        assert_eq!(
            s.set(&user("https://x.example/r")),
            Err(SourceError::NoCount)
        );
        let mut h = user("https://x.example/r?n={n}");
        h.key_header = Some("Cookie".into());
        assert_eq!(s.set(&h), Err(SourceError::BadHeader));
    }

    #[test]
    fn a_new_origin_unbinds_the_key() {
        let mut s = Sources::default();
        let mut c = user("https://a.example/r?n={n}");
        c.key_header = Some("x-key".into());
        assert!(s.set(&c).is_ok());
        let r = s.bind_key("mine").unwrap_or_default();
        assert!(s.get("mine").is_some_and(Source::key_bound));
        // Same origin, another path: still bound.
        let same = SourceChange {
            id: "mine".into(),
            url: Some("https://a.example/other?n={n}".into()),
            ..SourceChange::default()
        };
        assert_eq!(s.set(&same).map(|x| x.1), Ok(None));
        assert!(s.get("mine").is_some_and(Source::key_bound));
        let moved = SourceChange {
            id: "mine".into(),
            url: Some("https://b.example/r?n={n}".into()),
            ..SourceChange::default()
        };
        assert_eq!(s.set(&moved).map(|x| x.1), Ok(Some(r)));
        assert!(s.get("mine").is_some_and(|m| m.key_ref.is_none()));
    }

    #[test]
    fn presets_change_only_enabled_and_lfd_waits() {
        let mut s = Sources::default();
        let change = SourceChange {
            id: "anu".into(),
            url: Some("https://elsewhere.example?n={n}".into()),
            ..SourceChange::default()
        };
        assert_eq!(s.set(&change), Err(SourceError::PresetFixed));
        let on = SourceChange {
            id: "lfd".into(),
            enabled: Some(true),
            ..SourceChange::default()
        };
        assert_eq!(s.set(&on), Err(SourceError::NotYetAllowed));
        assert!(s.listed().all(|x| x.id != "lfd"));
        let org = SourceChange {
            id: "random-org".into(),
            enabled: Some(true),
            ..SourceChange::default()
        };
        assert!(s.set(&org).is_ok());
        let kept = s.settings();
        assert_eq!(kept.presets["random-org"].enabled, Some(true));
        assert_eq!(Sources::from_settings(&kept), s);
    }

    #[test]
    fn settings_round_trip_with_a_user_source() {
        let mut s = Sources::default();
        assert!(s.set(&user("https://a.example/r?n={n}")).is_ok());
        let kept = s.settings();
        let text = serde_json::to_string(&kept).unwrap_or_default();
        let back: SourcesSettings = serde_json::from_str(&text).unwrap_or_default();
        assert_eq!(Sources::from_settings(&back), s);
    }
}
