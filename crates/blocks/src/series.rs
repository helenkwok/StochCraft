//! Results over time: values recorded at fixed times in every replication, their mean across
//! replications with a 95% band at each time, and a warm-up suggested from them.
//!
//! One recorder serves every chart of a measure over a run: Welch's plot for choosing a warm-up
//! (M2.9), the results' charts over the run (M3.7) and, later, results over time for system
//! dynamics (M4). [`Compiled::record_every`] turns it on; a replication then records, at time
//! 0 and every interval after it up to its end, the state of each element just after
//! everything that happens at that instant has happened. Recording reads the statistics the
//! blocks keep anyway, between events, so it changes no event, no draw and no result.
//!
//! [`Compiled::record_every`]: crate::Compiled::record_every

use stochcraft_model::{ElementId, ElementKind, Experiment};
use stochcraft_stats::spread::quantile;
use stochcraft_stats::warmup::{MSER_BATCH, Truncation, mser5};
use stochcraft_stats::{Summary, Tally};

use crate::Report;

/// A quantity recorded over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Quantity {
    /// Entities waiting for a unit (servers and tasks with shared resources).
    QueueLength,
    /// Entities inside: arriving in transfer, waiting, in service or delayed, and held while
    /// blocked (as [`crate::Metric::WorkInProgress`] counts them).
    WorkInProgress,
    /// Units busy (servers; a pool's units in use).
    Busy,
    /// Entities that have entered since time 0, warm-up included (a source: created).
    Entered,
    /// Entities that have left since time 0, warm-up included (a sink: removed).
    Left,
}

impl Quantity {
    pub const ALL: [Quantity; 5] = [
        Quantity::QueueLength,
        Quantity::WorkInProgress,
        Quantity::Busy,
        Quantity::Entered,
        Quantity::Left,
    ];

    /// Its name in results, as the metric of the same meaning is named where there is one.
    pub fn name(self) -> &'static str {
        match self {
            Quantity::QueueLength => "queue_length",
            Quantity::WorkInProgress => "work_in_progress",
            Quantity::Busy => "busy",
            Quantity::Entered => "entered",
            Quantity::Left => "left",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.name() == s)
    }

    /// The quantities recorded for an element of kind `kind`: those the block keeps a state
    /// for.
    pub fn of_kind(kind: &ElementKind) -> &'static [Quantity] {
        use Quantity::*;
        match kind {
            ElementKind::Server { .. } => &[QueueLength, WorkInProgress, Busy, Entered, Left],
            ElementKind::SharedTask { .. } => &[QueueLength, WorkInProgress, Entered, Left],
            ElementKind::Delay { .. }
            | ElementKind::Batch { .. }
            | ElementKind::Conveyor { .. }
            | ElementKind::Transport { .. } => &[WorkInProgress, Entered, Left],
            // Splits each batch at once, holding nothing.
            ElementKind::Unbatch => &[Entered, Left],
            ElementKind::Source { .. } => &[Entered, Left],
            ElementKind::Sink { .. } => &[Entered],
            ElementKind::ExclusiveGateway { .. } | ElementKind::ParallelGateway => &[Entered, Left],
        }
    }

    /// The quantities recorded for a shared pool.
    pub const POOL: &'static [Quantity] = &[Quantity::Busy];
}

/// The most points a replication records: a `baseResultFrequency` far shorter than the run
/// (a second over a month) is taken in whole multiples until the run has at most this many.
pub const MAX_POINTS: usize = 1_000;

/// Points over the run when the experiment gives no result frequency: 100 intervals, so 101
/// points with time 0. MSER-5 then has 20 batches and places a warm-up to within 5% of the run.
pub const DEFAULT_INTERVALS: u32 = 100;

/// The interval to record at for `x`, in its time unit: its result frequency
/// ([`Experiment::result_frequency`], BPSim's `baseResultFrequency`) if it has one, taken in
/// whole multiples if that would give more than [`MAX_POINTS`] points; otherwise the warm-up
/// plus the duration over [`DEFAULT_INTERVALS`]. `None` for a run of no length.
pub fn record_interval(x: &Experiment) -> Option<f64> {
    let end = x.warmup.max(0.0) + x.duration.max(0.0);
    if !(end.is_finite() && end > 0.0) {
        return None;
    }
    let given = x
        .result_frequency
        .map(|secs| secs / x.time_unit.seconds())
        .filter(|f| f.is_finite() && *f > 0.0);
    Some(match given {
        Some(f) => {
            let points = end / f;
            if points > MAX_POINTS as f64 {
                // Whole multiples of the file's interval, so the points stay on its grid.
                f * (points / MAX_POINTS as f64).ceil()
            } else {
                f
            }
        }
        None => end / f64::from(DEFAULT_INTERVALS),
    })
}

/// One replication's recorded values.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Recording {
    /// The interval between recorded times.
    pub interval: f64,
    /// The times recorded at: 0, the interval, twice the interval, … up to the end (fewer when
    /// the stop condition ended the replication early).
    pub times: Vec<f64>,
    /// For each element, in the report's order (model order, then each shared pool), its
    /// quantities with one value per time.
    pub elements: Vec<Vec<(Quantity, Vec<f64>)>>,
}

/// One quantity's values across replications at each recorded time.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Band {
    /// The mean across the replications that reached the time.
    pub mean: Vec<f64>,
    /// The half-width of the mean's 95% Student-t confidence interval, as every other result
    /// gives (infinite where one replication reached the time): where the mean is.
    pub half_width: Vec<f64>,
    /// The 2.5th and 97.5th percentiles across replications (Hyndman and Fan's definition 7,
    /// as `results.distribution` uses): where 95% of single runs are.
    pub low: Vec<f64>,
    pub high: Vec<f64>,
}

/// One element's recorded quantities across replications.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementSeries {
    pub id: ElementId,
    pub quantities: Vec<(Quantity, Band)>,
}

impl ElementSeries {
    pub fn band(&self, q: Quantity) -> Option<&Band> {
        self.quantities
            .iter()
            .find(|(x, _)| *x == q)
            .map(|(_, b)| b)
    }
}

/// Results over time: each recorded quantity's mean and band across replications at each time.
#[derive(Debug, Clone, PartialEq)]
pub struct Series {
    pub interval: f64,
    /// The times, those of the replication that recorded the most.
    pub times: Vec<f64>,
    /// How many replications reached each time (fewer than all where the stop condition ended
    /// some early).
    pub n: Vec<u64>,
    pub elements: Vec<ElementSeries>,
    /// The warm-up MSER-5 suggests; `None` unless every replication recorded every time (a stop
    /// condition that ended one early leaves series of different lengths), or there is no
    /// task to judge.
    pub warmup: Option<WarmupSuggestion>,
}

impl Series {
    pub fn element(&self, id: &str) -> Option<&ElementSeries> {
        self.elements.iter().find(|e| e.id.as_str() == id)
    }
}

/// The quantity a warm-up is judged on: the number inside each task. It carries the start-up
/// whether or not a queue forms (a lightly loaded server starting empty fills its units before
/// any entity waits, and its queue length then stays near 0 throughout, saying nothing), and for
/// a single server it is the L of queueing theory.
pub const WARMUP_QUANTITY: Quantity = Quantity::WorkInProgress;

/// A warm-up suggested by MSER-5 (see [`stochcraft_stats::warmup`]) on the mean across
/// replications of [`WARMUP_QUANTITY`] at each task (servers and tasks with shared resources).
#[derive(Debug, Clone, PartialEq)]
pub struct WarmupSuggestion {
    /// The latest of the tasks' suggestions, in the model's time unit (0: no start-up to
    /// remove); `None` when every task's series is too short to tell.
    pub warmup: Option<f64>,
    /// The task it comes from.
    pub at: Option<ElementId>,
    /// Whether some task's series was still settling at the middle of the run, so that its
    /// start-up may last longer than half of it.
    pub too_short: bool,
    pub elements: Vec<ElementWarmup>,
}

impl WarmupSuggestion {
    /// Whether the start-up matters at some task ([`ElementWarmup::matters`]): what the
    /// `no_warmup` warning asks, and whether the suggestion is clearer than chance.
    pub fn matters(&self) -> bool {
        self.elements.iter().any(ElementWarmup::matters)
    }
}

/// MSER-5's suggestion for one task.
#[derive(Debug, Clone, PartialEq)]
pub struct ElementWarmup {
    pub id: ElementId,
    /// The suggested warm-up, the time of the first point kept; `None` when the series is too
    /// short to tell (then the shift is that of leaving out its first half).
    pub warmup: Option<f64>,
    /// How much leaving out the suggested warm-up moves the quantity's mean: in each
    /// replication, the mean of its recorded values over the whole run less their mean after
    /// the warm-up, summarised across replications with its 95% interval. `None` with no
    /// warm-up suggested.
    pub shift: Option<Summary>,
    /// How far the start of the run is from its second half: in each replication, the mean of
    /// the first batch of recorded values after time 0 (the [`MSER_BATCH`] values at the
    /// first interval and after it) less the mean of the second half, summarised across
    /// replications with its 95% interval. Fixed in advance, not chosen by MSER-5, so its
    /// interval is a fair test of whether the run starts away from where it settles
    /// ([`ElementWarmup::matters`]).
    pub start: Option<Summary>,
}

impl ElementWarmup {
    /// Whether the start-up matters: MSER-5 suggests a warm-up (or finds the series too short
    /// to place one), and the run starts lower than its second half by more than chance (the
    /// 95% interval of [`ElementWarmup::start`] lies below 0; never with a single replication,
    /// whose interval is unbounded). Runs start empty, so a start-up shows as the number inside
    /// starting low.
    ///
    /// MSER-5 alone is not enough: on a series with no start-up it still cuts something in
    /// about half of the runs, picking the stretch of noise that sets the start furthest apart
    /// from the rest, and its own difference then passes a 95% test in a fifth to a third of
    /// them. The start is tested on a window fixed in advance instead. Measured on M/M/1 queues
    /// started empty with no warm-up, 100 seeds each, with 3, 10 and 30 replications:
    /// - loads of 0.3, 0.5 and 0.7 over 5,000 time units (start-ups of at most a few dozen,
    ///   within the first interval): it matters in 3 to 9% of runs, false alarms;
    /// - a load of 0.9 over 2,000 (a start-up of several hundred): in 9, 50 and 94% of runs;
    ///   over 5,000: in 6, 24 and 55%. A start-up too small to tell from the noise of a few
    ///   replications is not reported; the suggestion itself always is.
    pub fn matters(&self) -> bool {
        let cut = self.warmup.is_none_or(|t| t > 0.0);
        let away = self
            .start
            .is_some_and(|s| s.half_width.is_finite() && s.mean < -s.half_width);
        cut && away
    }
}

/// Summarises the recordings of `reports` (`None` when none has one).
pub fn summarise_series(reports: &[Report]) -> Option<Series> {
    let recordings: Vec<&Recording> = reports
        .iter()
        .filter_map(|r| r.recording.as_ref())
        .collect();
    let longest = recordings.iter().max_by_key(|r| r.times.len())?;
    let points = longest.times.len();
    let n: Vec<u64> = (0..points)
        .map(|k| recordings.iter().filter(|r| r.times.len() > k).count() as u64)
        .collect();
    let first = reports.first()?;
    let elements: Vec<ElementSeries> = longest
        .elements
        .iter()
        .enumerate()
        .filter_map(|(i, quantities)| {
            let id = first.elements.get(i)?.id.clone();
            let quantities = quantities
                .iter()
                .enumerate()
                .map(|(j, (q, _))| {
                    let values = |k: usize| -> Vec<f64> {
                        recordings
                            .iter()
                            .filter_map(|r| r.elements.get(i)?.get(j)?.1.get(k).copied())
                            .collect()
                    };
                    (*q, band(points, values))
                })
                .collect();
            Some(ElementSeries { id, quantities })
        })
        .collect();
    let complete =
        recordings.len() == reports.len() && recordings.iter().all(|r| r.times.len() == points);
    let warmup = complete
        .then(|| suggest_warmup(first, &recordings, &longest.times, &elements))
        .flatten();
    Some(Series {
        interval: longest.interval,
        times: longest.times.clone(),
        n,
        elements,
        warmup,
    })
}

/// The band at each of `points` times, from the values `at(k)` there.
fn band(points: usize, at: impl Fn(usize) -> Vec<f64>) -> Band {
    let mut b = Band::default();
    for k in 0..points {
        let mut v = at(k);
        let s = Summary::of(&v);
        v.retain(|x| x.is_finite());
        v.sort_by(f64::total_cmp);
        b.mean.push(s.map_or(f64::NAN, |s| s.mean));
        b.half_width.push(s.map_or(f64::NAN, |s| s.half_width));
        b.low.push(quantile(&v, 0.025).unwrap_or(f64::NAN));
        b.high.push(quantile(&v, 0.975).unwrap_or(f64::NAN));
    }
    b
}

/// MSER-5 on each task's mean [`WARMUP_QUANTITY`] series (all of the same length).
fn suggest_warmup(
    first: &Report,
    recordings: &[&Recording],
    times: &[f64],
    elements: &[ElementSeries],
) -> Option<WarmupSuggestion> {
    let mut out = Vec::new();
    let mut too_short = false;
    for (i, e) in elements.iter().enumerate() {
        let kind = first.elements.get(i).map_or("", |r| r.kind);
        if kind != "server" && kind != "shared_task" {
            continue;
        }
        let Some(mean) = e.band(WARMUP_QUANTITY).map(|b| &b.mean) else {
            continue;
        };
        let Some(j) = recordings
            .first()?
            .elements
            .get(i)?
            .iter()
            .position(|(q, _)| *q == WARMUP_QUANTITY)
        else {
            continue;
        };
        let cut = match mser5(mean) {
            Some(Truncation::At { observations }) => Some(observations),
            Some(Truncation::TooShort) => {
                too_short = true;
                None
            }
            None => continue,
        };
        // Leaving out the first half when the series is too short to tell.
        let from = cut.unwrap_or(mean.len() / MSER_BATCH / 2 * MSER_BATCH);
        let shifts: Vec<f64> = recordings
            .iter()
            .filter_map(|r| {
                let y = &r.elements.get(i)?.get(j)?.1;
                Some(average(y)? - average(y.get(from..)?)?)
            })
            .collect();
        // The first batch after time 0 against the second half.
        let half = mean.len() / 2;
        let starts: Vec<f64> = recordings
            .iter()
            .filter_map(|r| {
                let y = &r.elements.get(i)?.get(j)?.1;
                Some(average(y.get(1..=MSER_BATCH)?)? - average(y.get(half..)?)?)
            })
            .collect();
        out.push(ElementWarmup {
            id: e.id.clone(),
            warmup: cut.and_then(|c| times.get(c).copied()),
            shift: (from > 0).then(|| Summary::of(&shifts)).flatten(),
            start: Summary::of(&starts),
        });
    }
    if out.is_empty() {
        return None;
    }
    let latest = out
        .iter()
        .filter_map(|w| w.warmup.map(|t| (t, &w.id)))
        .fold(
            None,
            |best: Option<(f64, &ElementId)>, (t, id)| match best {
                Some((b, _)) if b >= t => best,
                _ => Some((t, id)),
            },
        );
    Some(WarmupSuggestion {
        warmup: latest.map(|(t, _)| t),
        at: latest.map(|(_, id)| id.clone()),
        too_short,
        elements: out,
    })
}

fn average(y: &[f64]) -> Option<f64> {
    let mut t = Tally::new();
    y.iter().for_each(|v| t.add(*v));
    t.mean()
}
