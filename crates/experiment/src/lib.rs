//! Running a model many times.
//!
//! An [`Experiment`] runs independent replications of a model, each with its own random stream
//! derived from the experiment seed and the replication index only. Results come back in index
//! order, so they are identical whether the replications ran on one thread or sixteen. On native
//! targets replications run on scoped threads (one simulation per thread, nothing shared); on
//! `wasm32` they run one after another. In the browser, the engine hands replications to the
//! page's web workers instead (`stochcraft_engine::Worker`).

use std::panic::AssertUnwindSafe;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use stochcraft_random::{Rng, SplitMix64};
pub use stochcraft_stats::Summary;

pub mod select;

/// What a replication gets: its index and its own random stream.
pub struct Replication {
    pub index: u64,
    pub rng: Rng,
}

/// Why an experiment did not produce every result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExperimentError {
    /// A replication panicked; its index, if known.
    ReplicationPanicked { index: Option<u64> },
    /// [`Control::cancel`] was called before every replication had been started. A cancel that
    /// comes once the last replication has been started is too late to stop anything: the run
    /// is complete and its results are returned (the replications running then finish as they
    /// would have), the same on one thread as on many.
    Cancelled,
}

/// Watches and stops an experiment from another thread: how many replications have finished,
/// and a flag that stops workers from starting more. A replication already running finishes,
/// unless the model asks [`Control::is_cancelled`] as it runs (as compiled models can, through
/// `stochcraft_blocks::Compiled::run_watched`).
#[derive(Debug, Default)]
pub struct Control {
    cancelled: AtomicBool,
    done: AtomicU64,
}

impl Control {
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the experiment to stop: no more replications start, and it returns
    /// [`ExperimentError::Cancelled`], unless every replication had already been started, in
    /// which case the experiment is complete and returns its results.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Replications finished so far.
    pub fn done(&self) -> u64 {
        self.done.load(Ordering::Relaxed)
    }

    fn finished_one(&self) {
        self.done.fetch_add(1, Ordering::Relaxed);
    }
}

impl std::fmt::Display for ExperimentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExperimentError::ReplicationPanicked { index: Some(i) } => {
                write!(f, "replication {i} panicked")
            }
            ExperimentError::ReplicationPanicked { index: None } => {
                write!(f, "a replication panicked")
            }
            ExperimentError::Cancelled => write!(f, "the experiment was cancelled"),
        }
    }
}

impl std::error::Error for ExperimentError {}

/// A set of replications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Experiment {
    pub replications: u64,
    pub seed: u64,
    /// Worker threads; `None` uses the machine's available parallelism. Ignored on wasm32.
    pub threads: Option<usize>,
}

impl Experiment {
    pub fn new(replications: u64, seed: u64) -> Self {
        Self {
            replications,
            seed,
            threads: None,
        }
    }

    pub fn threads(mut self, threads: usize) -> Self {
        self.threads = Some(threads.max(1));
        self
    }

    /// The random stream of replication `index`: depends only on the seed and the index.
    pub fn stream(&self, index: u64) -> Rng {
        let mut sm = SplitMix64::new(self.seed ^ index.wrapping_mul(0xd1b5_4a32_d192_ed03));
        Rng::new(sm.next_u64())
    }

    /// Replication `index`: its index and random stream. Running replications one at a time
    /// with this (as the browser does, between frames) gives the same results as [`Self::run`].
    pub fn replication(&self, index: u64) -> Replication {
        Replication {
            index,
            rng: self.stream(index),
        }
    }

    /// Runs `model` once per replication and returns the results in index order.
    pub fn run<R, F>(&self, model: F) -> Result<Vec<R>, ExperimentError>
    where
        F: Fn(Replication) -> R + Sync,
        R: Send,
    {
        self.run_controlled(model, &Control::new())
    }

    /// As [`Self::run`], reporting progress to `control` and stopping when it is cancelled.
    pub fn run_controlled<R, F>(
        &self,
        model: F,
        control: &Control,
    ) -> Result<Vec<R>, ExperimentError>
    where
        F: Fn(Replication) -> R + Sync,
        R: Send,
    {
        self.run_range(0..self.replications, model, control)
    }

    /// As [`Self::run_controlled`], for replications `range.start` up to (not including)
    /// `range.end` only, in index order: how a sequential run adds a batch of replications to
    /// those it has, each exactly as a run of them all would make it.
    pub fn run_range<R, F>(
        &self,
        range: std::ops::Range<u64>,
        model: F,
        control: &Control,
    ) -> Result<Vec<R>, ExperimentError>
    where
        F: Fn(Replication) -> R + Sync,
        R: Send,
    {
        let workers = self.worker_count(range.end.saturating_sub(range.start));
        if workers <= 1 {
            let mut out = Vec::new();
            for index in range {
                if control.is_cancelled() {
                    return Err(ExperimentError::Cancelled);
                }
                let rep = self.replication(index);
                let r = std::panic::catch_unwind(AssertUnwindSafe(|| model(rep)))
                    .map_err(|_| ExperimentError::ReplicationPanicked { index: Some(index) })?;
                out.push(r);
                control.finished_one();
            }
            return Ok(out);
        }
        self.run_parallel(range, &model, workers, control)
    }

    /// Runs `model` (which returns one number per replication) and summarises the results.
    pub fn summarise<F>(&self, model: F) -> Result<Option<Summary>, ExperimentError>
    where
        F: Fn(Replication) -> f64 + Sync,
    {
        Ok(Summary::of(&self.run(model)?))
    }

    /// Worker threads for `replications` replications.
    fn worker_count(&self, replications: u64) -> usize {
        if cfg!(target_arch = "wasm32") {
            return 1;
        }
        let available = std::thread::available_parallelism().map_or(1, |n| n.get());
        let wanted = self.threads.unwrap_or(available).max(1);
        let reps = usize::try_from(replications).unwrap_or(usize::MAX);
        wanted.min(reps.max(1))
    }

    fn run_parallel<R, F>(
        &self,
        range: std::ops::Range<u64>,
        model: &F,
        workers: usize,
        control: &Control,
    ) -> Result<Vec<R>, ExperimentError>
    where
        F: Fn(Replication) -> R + Sync,
        R: Send,
    {
        let end = range.end;
        let count = end.saturating_sub(range.start);
        let next = AtomicU64::new(range.start);
        // `Err(index)` records a replication that panicked; the others still run.
        let results: Mutex<Vec<(u64, Result<R, ()>)>> = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..workers {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    while !control.is_cancelled() {
                        // Claim an index only while it is below `end`, so the counter never
                        // passes it (a plain increment would wrap at `u64::MAX`).
                        let mut seen = next.load(Ordering::Relaxed);
                        let claimed = loop {
                            if seen >= end {
                                break None;
                            }
                            match next.compare_exchange_weak(
                                seen,
                                seen + 1,
                                Ordering::Relaxed,
                                Ordering::Relaxed,
                            ) {
                                Ok(_) => break Some(seen),
                                Err(now) => seen = now,
                            }
                        };
                        let Some(index) = claimed else {
                            break;
                        };
                        let rep = self.replication(index);
                        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| model(rep)))
                            .map_err(|_| ());
                        local.push((index, outcome));
                        control.finished_one();
                    }
                    results
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .extend(local);
                });
            }
        });
        let mut out = results.into_inner().unwrap_or_else(|e| e.into_inner());
        out.sort_by_key(|(i, _)| *i);
        if let Some((index, _)) = out.iter().find(|(_, r)| r.is_err()) {
            return Err(ExperimentError::ReplicationPanicked {
                index: Some(*index),
            });
        }
        if u64::try_from(out.len()).ok() != Some(count) {
            return Err(if control.is_cancelled() {
                ExperimentError::Cancelled
            } else {
                ExperimentError::ReplicationPanicked { index: None }
            });
        }
        Ok(out.into_iter().filter_map(|(_, r)| r.ok()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_in_index_order_and_independent_of_threads() {
        let model = |mut r: Replication| (r.index, r.rng.next_u64());
        let one = Experiment::new(50, 9).threads(1).run(model);
        let four = Experiment::new(50, 9).threads(4).run(model);
        assert!(one.is_ok());
        assert_eq!(one, four);
        let v = one.unwrap_or_default();
        assert!(v.iter().enumerate().all(|(i, (idx, _))| *idx == i as u64));
        // Streams differ between replications and between seeds.
        assert_ne!(v[0].1, v[1].1);
        assert_ne!(
            Experiment::new(1, 9).stream(0).next_u64(),
            Experiment::new(1, 10).stream(0).next_u64()
        );
    }

    /// A batch of replications from the middle gives exactly what a whole run gives for them, on
    /// any number of threads: how a sequential run adds replications.
    #[test]
    fn a_range_of_replications_is_a_slice_of_the_whole_run() {
        let model = |mut r: Replication| (r.index, r.rng.next_u64());
        let whole = Experiment::new(12, 7)
            .threads(1)
            .run(model)
            .unwrap_or_default();
        for threads in [1, 3] {
            let x = Experiment::new(12, 7).threads(threads);
            let control = Control::new();
            let part = x.run_range(5..9, model, &control);
            assert_eq!(part.as_deref(), Ok(&whole[5..9]));
            assert_eq!(control.done(), 4);
            // Past the experiment's own count, too: only the seed and the index matter.
            let later = Experiment::new(4, 7).threads(threads);
            assert_eq!(
                later.run_range(8..12, model, &Control::new()).as_deref(),
                Ok(&whole[8..12])
            );
            assert_eq!(x.run_range(3..3, model, &Control::new()), Ok(vec![]));
        }
    }

    /// A range that ends at the last index runs exactly its replications on any number of
    /// threads: claiming indices never wraps the counter past the end.
    #[test]
    fn a_range_ending_at_the_last_index_stops_there() {
        let model = |r: Replication| r.index;
        for threads in [1, 2, 4] {
            let x = Experiment::new(2, 7).threads(threads);
            assert_eq!(
                x.run_range(u64::MAX - 2..u64::MAX, model, &Control::new()),
                Ok(vec![u64::MAX - 2, u64::MAX - 1])
            );
        }
    }

    #[test]
    fn zero_replications() {
        assert_eq!(
            Experiment::new(0, 1).threads(4).run(|r| r.index),
            Ok(vec![])
        );
        assert_eq!(Experiment::new(0, 1).summarise(|_| 1.0), Ok(None));
    }

    #[test]
    #[allow(clippy::panic)] // The point of the test is a model that panics.
    fn a_panicking_replication_is_an_error_not_a_crash() {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let res = Experiment::new(10, 1).threads(3).run(|r| {
            if r.index == 4 {
                std::panic::panic_any("boom");
            }
            r.index
        });
        std::panic::set_hook(prev);
        assert!(matches!(
            res,
            Err(ExperimentError::ReplicationPanicked { .. })
        ));
    }

    #[test]
    fn progress_and_cancellation() {
        for threads in [1, 3] {
            let control = Control::new();
            let done = Experiment::new(20, 1)
                .threads(threads)
                .run_controlled(|r| r.index, &control);
            assert_eq!(done.map(|v| v.len()), Ok(20));
            assert_eq!(control.done(), 20);
            // Cancelling from inside a replication: the ones running finish, no more start.
            let control = Control::new();
            let res = Experiment::new(1_000, 1).threads(threads).run_controlled(
                |r| {
                    if r.index == 5 {
                        control.cancel();
                    }
                    r.index
                },
                &control,
            );
            assert_eq!(res, Err(ExperimentError::Cancelled));
            assert!(control.done() < 1_000, "{}", control.done());
        }
        // A cancel during the last replication is too late: the run is complete, on any number
        // of threads.
        for threads in [1, 3] {
            let control = Control::new();
            let res = Experiment::new(20, 1).threads(threads).run_controlled(
                |r| {
                    if r.index == 19 {
                        control.cancel();
                    }
                    r.index
                },
                &control,
            );
            assert_eq!(res.map(|v| v.len()), Ok(20), "{threads} threads");
            assert!(control.is_cancelled());
        }
        // One at a time gives what a whole run gives.
        let x = Experiment::new(5, 42);
        let one_by_one: Vec<u64> = (0..5).map(|i| x.replication(i).rng.next_u64()).collect();
        assert_eq!(x.threads(2).run(|mut r| r.rng.next_u64()), Ok(one_by_one));
    }

    #[test]
    fn summarise_uniform_mean() {
        let s = Experiment::new(200, 3).summarise(|mut r| r.rng.uniform());
        let s = s.ok().flatten();
        assert!(
            s.is_some_and(|s| s.n == 200 && s.contains(0.5) && s.half_width < 0.06),
            "{s:?}"
        );
    }
}
