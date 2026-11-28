//! Where a run's random numbers come from (`run` and `run.start` take it as `random`).
//!
//! - `prng`, the default: the experiment's xoshiro256** streams from its seed, exactly as always.
//! - `seed_source`: the same streams, from a seed the caller drew from a source of its own (a
//!   quantum random number service, a public randomness beacon). The seed is the run's `seed`;
//!   the source, the time and the pulse it came from are only recorded, so the run replays from
//!   its record without calling out.
//! - `byte_file`: a file of random bytes in place of the generator. Replication i reads its own
//!   segment, bytes `offset + i·L` to `offset + (i+1)·L − 1`, eight at a time as little-endian
//!   `u64`s mapped strictly inside (0, 1) (`stochcraft_random::open_unit`), in the order its events
//!   draw. So a run gives the same results on threads, in slices and on workers. A run whose
//!   replications could read past the end of the file is refused before it starts; one whose
//!   segment runs out stops with `random_data_exhausted`, and no byte is ever read twice. The
//!   file's SHA-256 is recorded, and a `sha256` given with the file must match it.
//!
//! - `live`: bytes fetched from a random number source while the run goes, each replication's
//!   segment recorded in order into a file (see the `sources` module); the finished run's record
//!   is a `byte_file` run on that file. A sequential live run that met its target early recorded
//!   only the segments it fetched, so its record (marked by `live`) gives the `replications` it
//!   made, and the file is checked up front for those only (a journalled run, which has no count,
//!   for its first batch): a replay draws the same bytes, so it stops where the run did, and one
//!   that went further would stop with `random_data_exhausted` where the bytes run out.
//!
//! The record's `random` object can be given back as `random` to repeat the run: its extra
//! fields (`bytes_used`, ...) are accepted and ignored.
//!
//! Byte files serve process-model runs only. System dynamics, agent-based and Monte Carlo
//! models, and the commands that run a model many ways (sweeps, sensitivity, designs, sizing,
//! evaluation, optimisation), refuse them: their streams are keyed by seed and purpose, not read
//! from one cursor per replication.

use std::collections::BTreeMap;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use stochcraft_blocks::Compiled;
use stochcraft_experiment::Experiment;
use stochcraft_random::Rng;

use crate::{CommandError, ErrorCode};

/// Where the random numbers come from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RandomMode {
    /// The experiment's pseudo-random streams from its seed.
    #[default]
    Prng,
    /// The pseudo-random streams from a seed drawn from a source; the source is recorded.
    SeedSource,
    /// A file of random bytes, one segment per replication.
    ByteFile,
    /// Bytes fetched from a source while the run goes (`run.start` only): each replication's
    /// segment is fetched, up to four ahead, and recorded in order into `file`; a replication
    /// starts once its segment has arrived. The finished run's record is a `byte_file` run on
    /// that file, which repeats it without calling out.
    Live,
}

/// A run's `random` parameter, and the `random` object of its record.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomSpec {
    #[serde(default)]
    pub mode: RandomMode,
    /// `live`: the file the fetched bytes are recorded in (its manifest beside it as
    /// `<file>.json`).
    /// `byte_file`: the file of random bytes: a path, or a name given to
    /// `Engine::load_random_data` (how the browser passes a file's bytes).
    #[serde(default)]
    pub file: Option<String>,
    /// `byte_file`: the file's SHA-256 (hexadecimal, optionally after `sha256:`); a file with
    /// another is refused. The record gives it.
    #[serde(default)]
    pub sha256: Option<String>,
    /// `byte_file`: the byte the first replication's segment starts at; default 0.
    #[serde(default)]
    pub offset: u64,
    /// `byte_file`: each replication's segment length L, in bytes. By default from a pilot run
    /// (`random.pilot`): the most bytes a pilot replication read × 1.5, rounded up to 4 KiB.
    #[serde(default)]
    pub segment_bytes: Option<u64>,
    /// `seed_source`: where the seed came from (a service's name or address, with no key).
    /// `live`: the id of the source to fetch from (`random.sources`).
    #[serde(default)]
    pub source: Option<String>,
    /// `seed_source`: when it was drawn (ISO 8601).
    #[serde(default)]
    pub time: Option<String>,
    /// `seed_source`: the pulse or response it came from, such as a beacon pulse's URI.
    #[serde(default)]
    pub pulse: Option<String>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Value>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mapping: Option<Value>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_bytes: Option<Value>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_used: Option<Value>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub most_bytes_in_a_replication: Option<Value>,
    /// Filled in by the run record; accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pilot: Option<Value>,
    /// Filled in by the record of a live run (the source it fetched from): marks a file as a
    /// live run's recording, which is checked up front for `replications` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<Value>,
    /// A live run's recording: the replications the run made, which the file holds (by default
    /// the first batch's). Filled in by the record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replications: Option<u64>,
}

/// The bytes a run reads: `data`, replication i's segment starting at `offset + i·segment`.
#[derive(Debug, Clone)]
pub struct ByteFeed {
    data: Arc<[u8]>,
    offset: u64,
    segment: u64,
}

impl ByteFeed {
    pub fn new(data: Arc<[u8]>, offset: u64, segment: u64) -> Self {
        Self {
            data,
            offset,
            segment,
        }
    }

    /// Each replication's segment length.
    pub fn segment(&self) -> u64 {
        self.segment
    }

    /// The whole segments the data holds from the first replication's on.
    pub fn segments(&self) -> u64 {
        let have = u64::try_from(self.data.len()).unwrap_or(u64::MAX);
        have.saturating_sub(self.offset) / self.segment.max(1)
    }

    /// Replication `index`'s stream: its segment.
    pub fn stream(&self, index: u64) -> Rng {
        let start = index
            .checked_mul(self.segment)
            .and_then(|s| s.checked_add(self.offset))
            .unwrap_or(u64::MAX);
        Rng::from_bytes(Arc::clone(&self.data), start, self.segment)
    }

    /// The bytes the first `replications` replications read, from the first one's segment on;
    /// `None` when they would run past the end of the data, or their length cannot be counted.
    pub fn window(&self, replications: u64) -> Option<&[u8]> {
        let from = usize::try_from(self.offset).ok()?;
        let span = usize::try_from(replications.checked_mul(self.segment)?).ok()?;
        self.data.get(from..from.checked_add(span)?)
    }
}

/// The largest file of random bytes a run reads: 1 GiB on the desktop and the command line,
/// 256 MiB in the browser, where the page holds the whole file in memory beside the app. A larger
/// file is refused before it is read.
pub const MAX_FILE_BYTES: u64 = if cfg!(target_arch = "wasm32") {
    256 << 20
} else {
    1 << 30
};

/// The most bytes of a file a run on web workers sends with its task: 8 MiB. Every worker gets
/// the task (the bytes of every replication the run may have, as hexadecimal, so twice as long)
/// and decodes it, so each holds about three times the window, and a run has up to 16 workers:
/// 8 MiB comes to about 400 MiB in all. A run whose replications need more is refused before
/// it starts; it can run in slices instead, which keep one copy of the file.
pub const MAX_TASK_WINDOW_BYTES: u64 = 8 << 20;

/// The error for `bytes` of random data where at most `limit` are taken, with both in `details`.
pub(crate) fn too_large(what: &str, bytes: u64, limit: u64, advice: &str) -> CommandError {
    CommandError {
        code: ErrorCode::InvalidParams,
        message: format!("{what} has {bytes} bytes, more than the {limit} allowed: {advice}"),
        details: Some(json!({ "bytes": bytes, "limit_bytes": limit })),
    }
}

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

/// The bytes of hexadecimal `text`; `None` if it is not.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    };
    let t = text.as_bytes();
    if !t.len().is_multiple_of(2) {
        return None;
    }
    t.chunks(2)
        .map(|p| Some((digit(*p.first()?)? << 4) | digit(*p.get(1)?)?))
        .collect()
}

/// The SHA-256 of `bytes`, in hexadecimal.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    to_hex(&Sha256::digest(bytes))
}

/// How a run gets its random numbers, ready to run: the record's `random` object, and the bytes
/// of a run on a file.
pub(crate) struct Prepared {
    pub record: Value,
    pub feed: Option<Arc<ByteFeed>>,
}

/// The record's `random` object for pseudo-random streams from the experiment's seed.
pub(crate) fn prng_record() -> Value {
    json!({ "mode": "prng", "generator": "xoshiro256**" })
}

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

/// Reads the file a `byte_file` run names: bytes loaded under that name, else (not in the
/// browser) the file at that path. A file of more than `limit` bytes is refused, one on disk
/// before any of it is read.
pub(crate) fn read_file(
    loaded: &BTreeMap<String, Arc<[u8]>>,
    name: &str,
    limit: u64,
) -> Result<Arc<[u8]>, CommandError> {
    let advice = "give a smaller file, or the part of it the run reads";
    if let Some(d) = loaded.get(name) {
        let have = u64::try_from(d.len()).unwrap_or(u64::MAX);
        if have > limit {
            return Err(too_large(&format!("`{name}`"), have, limit, advice));
        }
        return Ok(Arc::clone(d));
    }
    if cfg!(target_arch = "wasm32") {
        return Err(invalid(format!(
            "no random-data file named `{name}` has been loaded"
        )));
    }
    let io = |e: std::io::Error| CommandError::new(ErrorCode::Io, format!("{name}: {e}"));
    let size = std::fs::metadata(name).map_err(io)?.len();
    if size > limit {
        return Err(too_large(&format!("`{name}`"), size, limit, advice));
    }
    let data = std::fs::read(name).map_err(io)?;
    // The file grew between the two looks.
    let have = u64::try_from(data.len()).unwrap_or(u64::MAX);
    if have > limit {
        return Err(too_large(&format!("`{name}`"), have, limit, advice));
    }
    Ok(Arc::from(data))
}

/// The most bytes in a replication for a segment, from a pilot's most: × 1.5, rounded up to
/// 4 KiB (at least 4 KiB).
pub fn segment_from_pilot(most: u64) -> u64 {
    let wanted = most.saturating_add(most.div_ceil(2));
    wanted.div_ceil(4096).max(1).saturating_mul(4096)
}

/// What a pilot found: the bytes each of its replications read, and the segment it suggests.
pub(crate) struct Pilot {
    pub bytes: Vec<u64>,
    pub segment: u64,
}

/// The replications a pilot runs by default: the experiment's first ten (or all, if fewer).
pub const PILOT_REPLICATIONS: u64 = 10;

/// Runs the first `count` replications of `experiment` as a run on a file would draw (one
/// cursor per replication), from pseudo-random numbers, counting the bytes each would read.
pub(crate) fn pilot(
    compiled: &Compiled,
    experiment: &Experiment,
    count: u64,
) -> Result<Pilot, CommandError> {
    let mut bytes = Vec::new();
    for i in 0..count.max(1) {
        let seed = experiment.stream(i).next_u64();
        let report = compiled
            .run(Rng::pilot(seed))
            .map_err(|e| crate::run::run_failed(i, &e))?;
        bytes.push(report.random_bytes);
    }
    let most = bytes.iter().copied().max().unwrap_or(0);
    Ok(Pilot {
        bytes,
        segment: segment_from_pilot(most),
    })
}

impl Pilot {
    pub fn json(&self) -> Value {
        json!({
            "replications": self.bytes.len(),
            "bytes": self.bytes,
            "most_bytes": self.bytes.iter().copied().max().unwrap_or(0),
            "segment_bytes": self.segment,
            "rule": "the most bytes a pilot replication read × 1.5, rounded up to 4 KiB; a pilot draws as a run on a file does (one cursor per replication), from pseudo-random numbers, so a run on the file can read more or fewer",
        })
    }
}

/// Prepares `spec` for a run of `compiled` with `experiment`, at most `max` replications, given
/// whether the caller gave the seed. Refuses a file that the replications could read past the
/// end of, or whose SHA-256 is not the one given.
pub(crate) fn prepare(
    spec: Option<RandomSpec>,
    seed_given: bool,
    loaded: &BTreeMap<String, Arc<[u8]>>,
    compiled: &Compiled,
    experiment: &Experiment,
    max: u64,
) -> Result<Prepared, CommandError> {
    let spec = spec.unwrap_or_default();
    match spec.mode {
        RandomMode::Prng => Ok(Prepared {
            record: prng_record(),
            feed: None,
        }),
        RandomMode::SeedSource => {
            if !seed_given {
                return Err(invalid(
                    "`seed_source` records where the run's seed came from: give that seed as `seed`",
                ));
            }
            let Some(source) = spec.source.filter(|s| !s.trim().is_empty()) else {
                return Err(invalid(
                    "`seed_source` needs the `source` the seed came from",
                ));
            };
            Ok(Prepared {
                record: json!({
                    "mode": "seed_source",
                    "generator": "xoshiro256**",
                    "source": source,
                    "time": spec.time,
                    "pulse": spec.pulse,
                }),
                feed: None,
            })
        }
        RandomMode::ByteFile => {
            let Some(name) = spec.file.filter(|f| !f.trim().is_empty()) else {
                return Err(invalid("`byte_file` needs the `file` of random bytes"));
            };
            let data = read_file(loaded, &name, MAX_FILE_BYTES)?;
            let sha = sha256_hex(&data);
            if let Some(given) = &spec.sha256 {
                let given = given.trim();
                let given = given.strip_prefix("sha256:").unwrap_or(given);
                if !given.eq_ignore_ascii_case(&sha) {
                    return Err(invalid(format!(
                        "`{name}` has SHA-256 {sha}, not the {given} given: it is not the file the run is to read"
                    )));
                }
            }
            let (segment, pilot) = match spec.segment_bytes {
                Some(0) => return Err(invalid("`segment_bytes` must be at least 8")),
                Some(l) => (l, None),
                None => {
                    let count = experiment.replications.clamp(1, PILOT_REPLICATIONS);
                    let p = pilot(compiled, experiment, count)?;
                    (p.segment, Some(p.json()))
                }
            };
            if segment < 8 {
                return Err(invalid("`segment_bytes` must be at least 8"));
            }
            let have = u64::try_from(data.len()).unwrap_or(u64::MAX);
            // A live run's recording holds the replications the run made: a replay stops where
            // the run did. Any other file must hold every replication the run may make.
            let checked = if spec.live.is_some() {
                spec.replications
                    .unwrap_or(experiment.replications)
                    .min(max)
            } else {
                max
            };
            let need = checked
                .checked_mul(segment)
                .and_then(|n| n.checked_add(spec.offset));
            if need.is_none_or(|n| n > have) {
                let need_text =
                    need.map_or_else(|| "more than can be counted".to_owned(), |n| n.to_string());
                return Err(invalid(format!(
                    "`{name}` has {have} bytes, but {checked} replications of {segment} bytes from byte {} need {need_text}: give a larger file, fewer replications or a shorter segment (the file is never read twice)",
                    spec.offset
                )));
            }
            let mut record = json!({
                "mode": "byte_file",
                "file": name,
                "sha256": sha,
                "file_bytes": have,
                "offset": spec.offset,
                "segment_bytes": segment,
                "mapping": "each replication i reads bytes offset + i × segment_bytes on, eight at a time as a little-endian u64 x, mapped to (2·(x >> 12) + 1)·2^-53, strictly inside (0, 1)",
            });
            if let Some(p) = pilot {
                record["pilot"] = p;
            }
            if let Some(live) = spec.live {
                record["live"] = live;
                record["replications"] = json!(checked);
            }
            Ok(Prepared {
                record,
                feed: Some(Arc::new(ByteFeed::new(data, spec.offset, segment))),
            })
        }
        // Planned by the engine, which has the sources.
        RandomMode::Live => Err(invalid("a live run is planned by the engine")),
    }
}

/// The commands that run a model in ways that cannot read a file of random bytes: they refuse
/// `random` with `byte_file`, and take it (and ignore it) otherwise.
const NO_BYTE_FILES: &[&str] = &[
    "sd.run",
    "agents.run",
    "mc.run",
    "mc.sobol",
    "experiment.sweep",
    "experiment.sensitivity",
    "experiment.design",
    "experiment.evaluate",
    "experiment.optimise",
    "experiment.size",
];

/// Checks the `random` parameter of a command that cannot read a file of random bytes, and
/// takes it out of `params` (the command itself does not know it).
pub(crate) fn screen(command: &str, params: &mut Value) -> Result<(), CommandError> {
    if !NO_BYTE_FILES.contains(&command) {
        return Ok(());
    }
    let Some(random) = params.as_object_mut().and_then(|p| p.remove("random")) else {
        return Ok(());
    };
    let spec: RandomSpec =
        serde_json::from_value(random).map_err(|e| invalid(format!("`random`: {e}")))?;
    if !matches!(spec.mode, RandomMode::ByteFile | RandomMode::Live) {
        return Ok(());
    }
    let why = match command {
        "sd.run" => {
            "a system dynamics model draws by the seed and each random function's place, not from one stream per replication"
        }
        "agents.run" => {
            "an agent-based model draws from a stream per agent and purpose, keyed by the seed"
        }
        "mc.run" | "mc.sobol" => {
            "a Monte Carlo model draws each input from a stream of its own (and samples them by Latin hypercube or Sobol points), keyed by the seed"
        }
        _ => "it runs the model at many settings with common random numbers keyed by the seed",
    };
    Err(CommandError::new(
        ErrorCode::Unsupported,
        format!(
            "`{command}` cannot read random numbers from a file: {why}. Random-data files are for `run` and `run.start` on a process model; use the seed here"
        ),
    ))
}

/// Where a run's random numbers came from, as far as pairing two runs goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Identity {
    /// The pseudo-random streams from this seed (also a seed drawn from a source): replication
    /// i's stream depends only on the seed and i.
    Seed(u64),
    /// The file with this SHA-256 (64 lowercase hexadecimal digits), replication i reading the
    /// `segment` bytes from `offset + i·segment`.
    File {
        sha256: String,
        offset: u64,
        segment: u64,
    },
}

impl Identity {
    /// As `results.compare` gives it.
    pub fn json(&self) -> Value {
        match self {
            Identity::Seed(seed) => json!({ "mode": "prng", "seed": seed }),
            Identity::File {
                sha256,
                offset,
                segment,
            } => json!({
                "mode": "byte_file",
                "sha256": sha256,
                "offset": offset,
                "segment_bytes": segment,
            }),
        }
    }

    /// The bytes of the file the first `replications` replications read, as `[from, to)`;
    /// `None` for a seed. Counted in 128 bits, so it never overflows.
    pub fn span(&self, replications: u64) -> Option<(u128, u128)> {
        match self {
            Identity::Seed(_) => None,
            Identity::File {
                offset, segment, ..
            } => {
                let from = u128::from(*offset);
                Some((from, from + u128::from(*segment) * u128::from(replications)))
            }
        }
    }
}

/// Where the random numbers of run results `results` (`which`, for messages) came from, for
/// pairing. Results without a record's `random`, or with a pseudo-random one, are pseudo-random
/// from their `seed`; a `byte_file` record must give the file's SHA-256, the offset and the
/// segment length, and any other mode is refused.
pub(crate) fn identity(results: &Value, which: &str) -> Result<Identity, CommandError> {
    let r = &results["record"]["random"];
    let seed = || {
        results["seed"]
            .as_u64()
            .map(Identity::Seed)
            .ok_or_else(|| invalid(format!("`{which}` has no `seed`: give a run's results")))
    };
    let mode = match r {
        Value::Null => return seed(),
        Value::Object(o) => o.get("mode").cloned().unwrap_or(Value::Null),
        _ => {
            return Err(invalid(format!(
                "`{which}`'s `record.random` is not an object"
            )));
        }
    };
    match mode.as_str() {
        None | Some("prng" | "seed_source") => seed(),
        Some("byte_file") => {
            let bad = |what: &str| {
                invalid(format!(
                    "`{which}` read a file of random bytes, but its `record.random` has {what}"
                ))
            };
            let sha = r["sha256"]
                .as_str()
                .ok_or_else(|| bad("no `sha256`"))?
                .trim();
            let sha = sha
                .strip_prefix("sha256:")
                .unwrap_or(sha)
                .to_ascii_lowercase();
            if sha.len() != 64 || !sha.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err(bad("a `sha256` that is not 64 hexadecimal digits"));
            }
            let offset = r["offset"]
                .as_u64()
                .ok_or_else(|| bad("no whole-number `offset`"))?;
            let segment = r["segment_bytes"]
                .as_u64()
                .filter(|l| *l >= 8)
                .ok_or_else(|| bad("no `segment_bytes` of at least 8"))?;
            Ok(Identity::File {
                sha256: sha,
                offset,
                segment,
            })
        }
        Some(other) => Err(invalid(format!(
            "`{which}`'s random numbers came from `{other}`, which cannot be compared"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_goes_both_ways() {
        let bytes = [0u8, 1, 0x7f, 0x80, 0xfe, 0xff];
        assert_eq!(to_hex(&bytes), "00017f80feff");
        assert_eq!(from_hex("00017F80feff").as_deref(), Some(&bytes[..]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }

    #[test]
    fn a_pilot_segment_is_half_as_much_again_rounded_up_to_4_kib() {
        assert_eq!(segment_from_pilot(0), 4096);
        assert_eq!(segment_from_pilot(2730), 4096);
        assert_eq!(segment_from_pilot(2731), 8192);
        assert_eq!(segment_from_pilot(10_000), 16_384);
    }

    #[test]
    fn a_window_covers_the_segments_of_the_replications_asked_for() {
        let feed = ByteFeed::new((0..100u8).collect::<Vec<u8>>().into(), 10, 16);
        assert_eq!(feed.window(2), Some(&(10..42u8).collect::<Vec<u8>>()[..]));
        // Five segments reach byte 90; six would run past the end, and no count overflows.
        assert_eq!(feed.window(5).map(<[u8]>::len), Some(80));
        assert_eq!(feed.window(6), None);
        assert_eq!(feed.window(u64::MAX), None);
    }

    #[test]
    fn a_file_over_the_limit_is_refused_before_it_is_read() {
        let mut loaded = BTreeMap::new();
        loaded.insert("data".to_owned(), Arc::<[u8]>::from(vec![0u8; 100]));
        assert!(read_file(&loaded, "data", 100).is_ok());
        let err = read_file(&loaded, "data", 99).err();
        let details = err.as_ref().and_then(|e| e.details.clone());
        assert_eq!(err.map(|e| e.code), Some(ErrorCode::InvalidParams));
        assert_eq!(details, Some(json!({ "bytes": 100, "limit_bytes": 99 })));
        if !cfg!(target_arch = "wasm32") {
            let path =
                std::env::temp_dir().join(format!("stochcraft-limit-{}.bin", std::process::id()));
            let written = std::fs::write(&path, [7u8; 64]).is_ok();
            let name = path.to_string_lossy().into_owned();
            let refused = read_file(&BTreeMap::new(), &name, 63).err();
            let read = read_file(&BTreeMap::new(), &name, 64).ok();
            let _ = std::fs::remove_file(&path);
            assert!(written);
            assert_eq!(
                refused.and_then(|e| e.details),
                Some(json!({ "bytes": 64, "limit_bytes": 63 }))
            );
            assert_eq!(read.map(|d| d.len()), Some(64));
            assert_eq!(MAX_FILE_BYTES, 1 << 30);
        }
    }

    #[test]
    fn a_random_identity_is_checked_and_normalised() {
        let sha = "AB".repeat(32);
        let file = |random: Value| json!({ "seed": 5, "record": { "random": random } });
        assert_eq!(
            identity(&json!({ "seed": 5 }), "a").ok(),
            Some(Identity::Seed(5))
        );
        assert_eq!(
            identity(&file(json!({ "mode": "seed_source", "source": "x" })), "a").ok(),
            Some(Identity::Seed(5))
        );
        let good = file(
            json!({ "mode": "byte_file", "sha256": format!("sha256:{sha}"),
                                 "offset": 0, "segment_bytes": 4096 }),
        );
        assert_eq!(
            identity(&good, "a").ok(),
            Some(Identity::File {
                sha256: "ab".repeat(32),
                offset: 0,
                segment: 4096
            })
        );
        // Records that cannot say which bytes were read are refused, not taken as equal.
        for bad in [
            json!({ "mode": "byte_file" }),
            json!({ "mode": "byte_file", "sha256": null, "offset": 0, "segment_bytes": 4096 }),
            json!({ "mode": "byte_file", "sha256": "abc", "offset": 0, "segment_bytes": 4096 }),
            json!({ "mode": "byte_file", "sha256": sha, "offset": -1, "segment_bytes": 4096 }),
            json!({ "mode": "byte_file", "sha256": sha, "offset": 0, "segment_bytes": 4 }),
            json!({ "mode": "byte_file", "sha256": sha, "offset": 0 }),
            json!({ "mode": "dice" }),
            json!("prng"),
        ] {
            assert_eq!(
                identity(&file(bad.clone()), "a").err().map(|e| e.code),
                Some(ErrorCode::InvalidParams),
                "{bad}"
            );
        }
        assert!(identity(&json!({}), "a").is_err());
    }
}
