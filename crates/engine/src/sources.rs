//! Random number sources (`random.sources`, `random.source.set`, `random.estimate`,
//! `random.seed.draw`, `random.fetch`, `random.status`) and live runs, which fetch each
//! replication's segment while the run goes.
//!
//! The app gives the engine its fetching and its key store ([`crate::Engine::set_random_net`]).
//! Keys never pass through a command: the window binds them ([`crate::Engine::set_random_key`]),
//! and commands see only whether a source has one. A draw or a fetch is a job the caller polls
//! with `random.status` (natively, `wait` polls it until it is done); none is journalled, so a
//! replay never calls out.

use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use stochcraft_qrng::record::Manifest;
use stochcraft_qrng::{Download, Source, estimate, seed};

use crate::commands::parse;
use crate::{CommandError, Engine, ErrorCode};

/// Segments a live run fetches ahead of the replication it is running.
pub const AHEAD: u64 = 4;

fn invalid(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidParams, message)
}

fn failed(e: impl std::fmt::Display) -> CommandError {
    CommandError::new(ErrorCode::RandomSourceFailed, e.to_string())
}

/// The manifest written beside a recorded `file`: none in the browser, which keeps the bytes in
/// the page under that name and writes no file.
fn manifest_written(file: &str) -> Option<String> {
    (!cfg!(target_arch = "wasm32")).then(|| stochcraft_qrng::record::manifest_path(file))
}

/// What a draw or a fetch is for.
#[derive(Debug)]
enum Purpose {
    Seed,
    File { file: String },
}

/// A draw or a fetch in progress, or its outcome.
#[derive(Debug)]
pub(crate) struct RandomJob {
    purpose: Purpose,
    download: Download,
    outcome: Option<Result<Value, CommandError>>,
}

/// A live run's bytes: fetched ahead of the replications, recorded in order into `file`.
#[derive(Debug)]
pub(crate) struct LiveFeed {
    pub download: Download,
    pub segment: u64,
    pub max: u64,
    pub file: String,
    /// The file has been created (emptied, if it was there).
    created: bool,
    /// The bytes written to the file so far: the first `committed` of the download's.
    committed: usize,
    finished: bool,
}

impl LiveFeed {
    /// Replication `index`'s segment has arrived.
    pub fn ready(&self, index: u64) -> bool {
        (self.download.data().len() as u64) >= index.saturating_add(1).saturating_mul(self.segment)
    }

    /// Replication `index`'s segment, once it has arrived.
    pub fn segment_bytes(&self, index: u64) -> Option<Arc<[u8]>> {
        let from = usize::try_from(index.checked_mul(self.segment)?).ok()?;
        let to = from.checked_add(usize::try_from(self.segment).ok()?)?;
        self.download.data().get(from..to).map(Arc::from)
    }

    /// The bytes recorded in the file (all fetched, in the browser, where nothing is written).
    fn recorded(&self) -> &[u8] {
        let data = self.download.data();
        &data[..self.committed.min(data.len())]
    }

    /// The record's `random` for the finished run of `replications`: a run on the recorded file,
    /// which repeats it. The file holds at least their segments (a sequential run that stopped
    /// early holds no more than it fetched), and the record says so.
    pub fn record(&self, replications: u64) -> Value {
        let data = self.recorded();
        let source = self.download.source();
        json!({
            "mode": "byte_file",
            "file": self.file,
            "sha256": stochcraft_qrng::sha256_hex(data),
            "file_bytes": data.len(),
            "offset": 0,
            "segment_bytes": self.segment,
            "replications": replications,
            "mapping": "each replication i reads bytes offset + i × segment_bytes on, eight at a time as a little-endian u64 x, mapped to (2·(x >> 12) + 1)·2^-53, strictly inside (0, 1)",
            "live": {
                "source": source.id,
                "name": source.name,
                "url": source.shown_url(),
                "requests": self.download.requests(),
                "fetched_ahead": AHEAD,
                "manifest": manifest_written(&self.file),
            },
        })
    }
}

/// `random.source.set`'s parameters are a [`stochcraft_qrng::SourceChange`].
/// `random.estimate`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomEstimate {
    pub source: String,
    /// `seed` (one draw), `file` or `live` (the default): the bytes the experiment's runs read.
    #[serde(default)]
    pub mode: Option<String>,
    /// For a file or a live run: replications (by default the experiment's, or its sequential
    /// maximum).
    #[serde(default)]
    pub replications: Option<u64>,
    /// For a file or a live run: each replication's segment (by default from a pilot).
    #[serde(default)]
    pub segment_bytes: Option<u64>,
}

/// `random.seed.draw`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomSeedDraw {
    pub source: String,
    /// Wait until the draw is done (not in the browser); otherwise poll `random.status`.
    #[serde(default)]
    pub wait: Option<bool>,
}

/// `random.fetch`'s parameters.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomFetch {
    pub source: String,
    /// The file to record the bytes in (in the browser, the name they are kept under), with its
    /// manifest beside it as `<file>.json`.
    pub file: String,
    /// The bytes to fetch; or give `replications` (and `segment_bytes`, else from a pilot).
    #[serde(default)]
    pub bytes: Option<u64>,
    #[serde(default)]
    pub replications: Option<u64>,
    #[serde(default)]
    pub segment_bytes: Option<u64>,
    #[serde(default)]
    pub wait: Option<bool>,
}

/// `random.status`'s parameters.
#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RandomStatus {
    /// Stop the draw or fetch in progress (a fetch keeps what it has fetched).
    #[serde(default)]
    pub cancel: bool,
}

fn source_json(e: &Engine, s: &Source) -> Value {
    let key = match &s.key_header {
        None => "not_needed",
        Some(_) if e.key_for(s).is_some() => "present",
        Some(_) => "absent",
    };
    json!({
        "id": s.id,
        "name": s.name,
        "kind": s.kind,
        "url": s.shown_url(),
        "enabled": s.enabled,
        "preset": s.preset,
        "seed_only": s.seed_only,
        "desktop_only": s.desktop_only,
        "key": key,
        "bytes_per_request": s.bytes_per_request(),
        "min_interval_ms": s.min_interval_ms,
        "free_requests_per_month": s.free_requests_per_month,
    })
}

impl Engine {
    /// The key kept for `source`, when it sends one and has one bound to its origin.
    pub(crate) fn key_for(&self, source: &Source) -> Option<stochcraft_qrng::Secret> {
        if !source.key_bound() {
            return None;
        }
        self.keys.get(source.key_ref.as_deref()?)
    }

    /// The source `id`, ready for `use_`: enabled, and reachable here.
    pub(crate) fn usable_source(&self, id: &str, for_bytes: bool) -> Result<Source, CommandError> {
        let s = self.sources.get(id).filter(|s| !s.hidden).ok_or_else(|| {
            invalid(format!(
                "no random number source `{id}` (see random.sources)"
            ))
        })?;
        if !s.enabled {
            return Err(invalid(format!(
                "`{id}` is disabled: enable it with random.source.set"
            )));
        }
        if s.desktop_only && cfg!(target_arch = "wasm32") {
            return Err(CommandError::new(
                ErrorCode::Unsupported,
                format!("`{id}` allows no requests from a browser page: use the desktop app"),
            ));
        }
        if for_bytes && s.seed_only {
            return Err(invalid(format!(
                "`{id}` serves seeds only (a few bytes a minute), not files or live runs"
            )));
        }
        Ok(s.clone())
    }

    fn download(&self, source: Source, bytes: u64) -> Result<Download, CommandError> {
        let key = self.key_for(&source);
        Download::new(source, key, bytes).map_err(failed)
    }

    /// The bytes a file or live run of the experiment reads: its replications (or `given`) ×
    /// its segment (or a pilot's).
    fn run_bytes(
        &self,
        replications: Option<u64>,
        segment: Option<u64>,
    ) -> Result<(u64, u64), CommandError> {
        let plan = self.plan(None, None, None, false, None, false, None)?;
        let n = replications
            .unwrap_or_else(|| plan.max_replications())
            .max(1);
        let l = match segment {
            Some(l) if l >= 8 => l,
            Some(_) => return Err(invalid("`segment_bytes` must be at least 8")),
            None => {
                let count = plan
                    .experiment
                    .replications
                    .clamp(1, crate::random::PILOT_REPLICATIONS);
                crate::random::pilot(&plan.compiled, &plan.experiment, count)?.segment
            }
        };
        Ok((n, l))
    }

    fn start_random_job(
        &mut self,
        purpose: Purpose,
        download: Download,
        wait: Option<bool>,
    ) -> Result<Value, CommandError> {
        let wait = wait.unwrap_or(!cfg!(target_arch = "wasm32"));
        if wait && cfg!(target_arch = "wasm32") {
            return Err(CommandError::new(
                ErrorCode::Unsupported,
                "the browser cannot wait: give `wait: false` and poll random.status",
            ));
        }
        // One acquisition at a time: a new one would drop the bytes of the one in progress.
        // That one is stepped first, so one that has just finished or failed gives way.
        if self
            .random_job
            .as_ref()
            .is_some_and(|j| j.outcome.is_none())
        {
            let _ = self.random_status_now(false);
        }
        if self
            .random_job
            .as_ref()
            .is_some_and(|j| j.outcome.is_none())
        {
            return Err(CommandError::new(
                ErrorCode::Conflict,
                "a draw or fetch is in progress: poll random.status until it is done, or stop it with random.status `cancel: true`",
            ));
        }
        self.random_job = Some(RandomJob {
            purpose,
            download,
            outcome: None,
        });
        let mut status = self.random_status_now(false);
        while wait && status["state"] == "running" {
            let pause = self
                .random_job
                .as_ref()
                .map_or(0, |j| j.download.wait_ms(self.fetch.now_ms()))
                .clamp(10, 200);
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(std::time::Duration::from_millis(pause));
            let _ = pause;
            status = self.random_status_now(false);
        }
        match status.get("error") {
            Some(err) if wait => Err(CommandError {
                code: ErrorCode::RandomSourceFailed,
                message: err["message"]
                    .as_str()
                    .unwrap_or("the fetch failed")
                    .to_owned(),
                details: None,
            }),
            _ => Ok(status),
        }
    }

    /// Steps the draw or fetch in progress and says where it is.
    pub(crate) fn random_status_now(&mut self, cancel: bool) -> Value {
        let Some(mut job) = self.random_job.take() else {
            return json!({ "state": "idle" });
        };
        if job.outcome.is_none() {
            if cancel {
                job.outcome = Some(Err(CommandError::new(
                    ErrorCode::Cancelled,
                    "the fetch was cancelled",
                )));
                if let Purpose::File { file } = &job.purpose {
                    let _ = self.record_bytes(file, &job.download);
                }
            } else {
                job.download.step(self.fetch.as_mut());
                if let Some(e) = job.download.error() {
                    job.outcome = Some(Err(failed(e)));
                } else if job.download.complete() {
                    job.outcome = Some(self.finish_random_job(&job));
                }
            }
        }
        let d = &job.download;
        let kind = match job.purpose {
            Purpose::Seed => "seed",
            Purpose::File { .. } => "file",
        };
        let mut v = json!({
            "kind": kind,
            "source": d.source().id,
            "bytes": d.data().len(),
            "target": d.target(),
            "requests": d.requests(),
        });
        match &job.outcome {
            None => {
                v["state"] = json!("running");
                v["wait_ms"] = json!(d.wait_ms(self.fetch.now_ms()));
            }
            Some(Ok(r)) => {
                v["state"] = json!("finished");
                v["result"] = r.clone();
            }
            Some(Err(e)) => {
                v["state"] = json!(if e.code == ErrorCode::Cancelled {
                    "cancelled"
                } else {
                    "failed"
                });
                v["error"] = e.to_json();
            }
        }
        self.random_job = Some(job);
        v
    }

    fn finish_random_job(&mut self, job: &RandomJob) -> Result<Value, CommandError> {
        let d = &job.download;
        let source = d.source();
        let health = stochcraft_qrng::health_check(d.data());
        match &job.purpose {
            Purpose::Seed => {
                let chunk = d.chunks().first();
                let s = seed::from_bytes(d.data());
                Ok(json!({
                    "seed": s,
                    "random": {
                        "mode": "seed_source",
                        "source": format!("{} ({})", source.name, source.shown_url()),
                        "time": chunk.map(|c| c.time.clone()),
                        "pulse": chunk.and_then(|c| c.pulse.clone()),
                    },
                    "rule": seed::RULE,
                    "bytes": d.data().len(),
                    "health": health,
                }))
            }
            Purpose::File { file } => {
                let manifest = self.record_bytes(file, d)?;
                Ok(json!({
                    "file": file,
                    "manifest": manifest_written(file),
                    "bytes": d.data().len(),
                    "sha256": manifest.sha256,
                    "requests": d.requests(),
                    "health": manifest.health,
                    "random": { "mode": "byte_file", "file": file, "sha256": manifest.sha256 },
                }))
            }
        }
    }

    /// Writes `d`'s bytes to `file` and its manifest beside it (in the browser, keeps them under
    /// that name for runs to read).
    fn record_bytes(&mut self, file: &str, d: &Download) -> Result<Manifest, CommandError> {
        let manifest = Manifest::new(d.source(), d.data(), d.chunks(), d.requests());
        if cfg!(target_arch = "wasm32") {
            self.load_random_data(file, d.data().to_vec());
        } else {
            let io = |e: std::io::Error| CommandError::new(ErrorCode::Io, format!("{file}: {e}"));
            std::fs::write(file, d.data()).map_err(io)?;
            std::fs::write(
                stochcraft_qrng::record::manifest_path(file),
                manifest.to_json(),
            )
            .map_err(io)?;
        }
        Ok(manifest)
    }

    /// A live run's feed: steps its download towards `AHEAD` segments past `next`, appends what
    /// arrived to its file, and once the run is over writes the manifest. An error stops it.
    pub(crate) fn step_live(
        &mut self,
        feed: &Arc<Mutex<LiveFeed>>,
        next: u64,
        over: bool,
    ) -> Result<(), CommandError> {
        let mut f = feed.lock().unwrap_or_else(|e| e.into_inner());
        if f.finished {
            return Ok(());
        }
        if !over {
            let want = next
                .saturating_add(AHEAD)
                .min(f.max)
                .saturating_mul(f.segment);
            f.download.set_target(want);
            f.download.step(self.fetch.as_mut());
        }
        let file = f.file.clone();
        let io = |e: std::io::Error| CommandError::new(ErrorCode::Io, format!("{file}: {e}"));
        if cfg!(target_arch = "wasm32") {
            f.committed = f.download.data().len();
        } else if !f.created || f.committed < f.download.data().len() {
            // Appends what is not yet in the file. A write that fails counts the bytes that
            // landed, so the next step (or the end of the run) writes only those still pending.
            let f = &mut *f;
            let mut out = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(f.committed > 0)
                .truncate(f.committed == 0)
                .open(&file)
                .map_err(io)?;
            f.created = true;
            stochcraft_qrng::record::append_pending(&mut out, f.download.data(), &mut f.committed)
                .map_err(io)?;
        }
        if over {
            f.finished = true;
            let d = &f.download;
            // The bytes in the file, which are every byte fetched once the writes succeeded.
            let manifest = Manifest::new(d.source(), f.recorded(), d.chunks(), d.requests());
            if cfg!(target_arch = "wasm32") {
                let data = d.data().to_vec();
                drop(f);
                self.load_random_data(&file, data);
            } else {
                std::fs::write(
                    stochcraft_qrng::record::manifest_path(&file),
                    manifest.to_json(),
                )
                .map_err(io)?;
            }
            return Ok(());
        }
        match f.download.error() {
            Some(e) => Err(failed(e)),
            None => Ok(()),
        }
    }

    /// The feed of a live run of `max` replications of `segment` bytes from `source_id`,
    /// recorded into `file`.
    pub(crate) fn live_feed(
        &self,
        source_id: &str,
        file: &str,
        segment: u64,
        max: u64,
    ) -> Result<Arc<Mutex<LiveFeed>>, CommandError> {
        if file.trim().is_empty() {
            return Err(invalid(
                "a live run needs the `file` to record its bytes in",
            ));
        }
        let source = self.usable_source(source_id, true)?;
        let download = self.download(source, AHEAD.min(max).saturating_mul(segment))?;
        Ok(Arc::new(Mutex::new(LiveFeed {
            download,
            segment,
            max,
            file: file.to_owned(),
            created: false,
            committed: 0,
            finished: false,
        })))
    }
}

pub(crate) fn random_sources(e: &mut Engine, _: Value) -> Result<Value, CommandError> {
    let list: Vec<Value> = e.sources.listed().map(|s| source_json(e, s)).collect();
    Ok(json!({
        "sources": list,
        "key_storage": e.keys.warning(),
    }))
}

pub(crate) fn random_source_set(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let change: stochcraft_qrng::SourceChange = parse(p)?;
    let (now, dropped) = e
        .sources
        .set(&change)
        .map_err(|err| invalid(err.to_string()))?;
    if let Some(r) = dropped {
        e.keys.delete(&r);
    }
    e.sources_revision += 1;
    Ok(match now {
        Some(s) => json!({ "source": source_json(e, &s) }),
        None => json!({ "removed": change.id }),
    })
}

pub(crate) fn random_estimate(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RandomEstimate = parse(p)?;
    let mode = p.mode.as_deref().unwrap_or("live");
    let source = e.usable_source(&p.source, mode != "seed")?;
    let (bytes, replications, segment) = match mode {
        "seed" => (estimate::SEED_BYTES, None, None),
        "file" | "live" => {
            let (n, l) = e.run_bytes(p.replications, p.segment_bytes)?;
            (estimate::run_bytes(n, l), Some(n), Some(l))
        }
        other => {
            return Err(invalid(format!(
                "`mode` is `seed`, `file` or `live`, not `{other}`"
            )));
        }
    };
    let est = estimate(&source, bytes);
    let mut v = serde_json::to_value(&est).unwrap_or_default();
    v["source"] = json!(source.id);
    v["mode"] = json!(mode);
    v["replications"] = json!(replications);
    v["segment_bytes"] = json!(segment);
    if est.within_free_tier == Some(false) {
        v["note"] = json!(
            "more requests than the source's free tier allows in a month: a run this size needs its paid tier, or a seed drawn from it"
        );
    }
    Ok(v)
}

pub(crate) fn random_seed_draw(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RandomSeedDraw = parse(p)?;
    let source = e.usable_source(&p.source, false)?;
    let bytes = if source.seed_only {
        source.bytes_per_request()
    } else {
        estimate::SEED_BYTES
    };
    let d = e.download(source, bytes)?;
    e.start_random_job(Purpose::Seed, d, p.wait)
}

pub(crate) fn random_fetch(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RandomFetch = parse(p)?;
    if p.file.trim().is_empty() {
        return Err(invalid("`file` names where the bytes go"));
    }
    let source = e.usable_source(&p.source, true)?;
    let bytes = match (p.bytes, p.replications) {
        (Some(b), None) if b > 0 => b,
        (None, Some(_)) => {
            let (n, l) = e.run_bytes(p.replications, p.segment_bytes)?;
            estimate::run_bytes(n, l)
        }
        _ => {
            return Err(invalid(
                "give `bytes` (above 0) or `replications`, not both",
            ));
        }
    };
    if bytes > 1 << 30 {
        return Err(invalid("a fetch takes at most 1 GiB"));
    }
    let d = e.download(source, bytes)?;
    e.start_random_job(Purpose::File { file: p.file }, d, p.wait)
}

pub(crate) fn random_status(e: &mut Engine, p: Value) -> Result<Value, CommandError> {
    let p: RandomStatus = parse(p)?;
    Ok(e.random_status_now(p.cancel))
}
