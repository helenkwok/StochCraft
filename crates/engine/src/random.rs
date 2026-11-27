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
//!   is a `byte_file` run on that file.
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
    /// Filled in by the record of a live run (the source it fetched from); accepted and ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<Value>,
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

    /// Replication `index`'s stream: its segment.
    pub fn stream(&self, index: u64) -> Rng {
        let start = index
            .checked_mul(self.segment)
            .and_then(|s| s.checked_add(self.offset))
            .unwrap_or(u64::MAX);
        Rng::from_bytes(Arc::clone(&self.data), start, self.segment)
    }

    /// The bytes the first `replications` replications read, from the first one's segment on.
    pub fn window(&self, replications: u64) -> &[u8] {
        let len = self.data.len();
        let from = usize::try_from(self.offset).unwrap_or(len).min(len);
        let span = replications.saturating_mul(self.segment);
        let to = usize::try_from(self.offset.saturating_add(span))
            .unwrap_or(len)
            .min(len);
        self.data.get(from..to).unwrap_or_default()
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
/// browser) the file at that path.
fn read_file(loaded: &BTreeMap<String, Arc<[u8]>>, name: &str) -> Result<Arc<[u8]>, CommandError> {
    if let Some(d) = loaded.get(name) {
        return Ok(Arc::clone(d));
    }
    if cfg!(target_arch = "wasm32") {
        return Err(invalid(format!(
            "no random-data file named `{name}` has been loaded"
        )));
    }
    std::fs::read(name)
        .map(Arc::from)
        .map_err(|e| CommandError::new(ErrorCode::Io, format!("{name}: {e}")))
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
            let data = read_file(loaded, &name)?;
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
            let need = max
                .checked_mul(segment)
                .and_then(|n| n.checked_add(spec.offset));
            if need.is_none_or(|n| n > have) {
                let need_text =
                    need.map_or_else(|| "more than can be counted".to_owned(), |n| n.to_string());
                return Err(invalid(format!(
                    "`{name}` has {have} bytes, but {max} replications of {segment} bytes from byte {} need {need_text}: give a larger file, fewer replications or a shorter segment (the file is never read twice)",
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

/// What a run's results say about where their random numbers came from, for pairing: two runs
/// meet the same random numbers replication by replication when this and their replications are
/// the same. Results without a record's `random` are pseudo-random from their `seed`.
pub(crate) fn identity(results: &Value) -> Value {
    let r = &results["record"]["random"];
    if r["mode"] == "byte_file" {
        json!({
            "mode": "byte_file",
            "sha256": r["sha256"],
            "offset": r["offset"],
            "segment_bytes": r["segment_bytes"],
        })
    } else {
        json!({ "mode": "prng", "seed": results["seed"] })
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
        assert_eq!(feed.window(2), &(10..42u8).collect::<Vec<u8>>()[..]);
        assert_eq!(feed.window(100).len(), 90);
    }
}
