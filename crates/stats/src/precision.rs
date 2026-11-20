//! How precisely the replications pin a result down, and how many replications to run until they
//! do: the rule both the results' "How sure are these numbers?" and sequential runs use.
//!
//! A result's precision is the half-width of its 95% Student-t interval against a target
//! ([`Scale`]): a share of its mean for a time or a count ([`SETTLED`] by default), percentage
//! points for a share such as a utilisation ([`SETTLED_POINTS`]), or an absolute half-width. Its
//! [`score`] is the half-width over that target, so 1 or less is settled.
//!
//! A sequential run ([`verdict`], [`next_count`]) adds replications in batches until every
//! result it watches is settled or it reaches its maximum, after the fully sequential procedure
//! of Hoad, Robinson and Davies (2011), "Automated selection of the number of replications for a
//! discrete-event simulation", *Journal of the Operational Research Society* 62(11), with two
//! guards: a result whose replications all gave the same value is never settled by that alone
//! (its spread may be a rare event not yet seen), and a share of a mean at or near zero is never
//! met. The interval after such adaptive stopping is an estimate of precision; its coverage is
//! not guaranteed to be 95%.

/// A time's or a count's 95% half-width as a share of its mean at which it is settled.
pub const SETTLED: f64 = 0.05;

/// A share's (a utilisation, a share lost) 95% half-width in percentage points at which it is
/// settled. Against its own mean a small share always looks unsure (1.7% ± 0.6 points is ± 32% of
/// itself), so shares are judged in points.
pub const SETTLED_POINTS: f64 = 1.0;

/// What a result's half-width is judged against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scale {
    /// A share of the mean's size (0.05 is ± 5%).
    Relative(f64),
    /// Percentage points, for a result that is a share between 0 and 1 (1.0 is ± 1 point).
    Points(f64),
    /// A half-width in the result's own unit.
    Absolute(f64),
}

impl Scale {
    /// The target's size, which must be above 0 and finite to be met.
    fn size(self) -> f64 {
        match self {
            Scale::Relative(x) | Scale::Points(x) | Scale::Absolute(x) => x,
        }
    }
}

/// A half-width of `half_width` around `mean` over the target `scale`: 1 or less is settled.
/// `None` when there is no finite interval, the target is not above 0, or for a relative target
/// with a mean of zero (no share of zero can be met).
pub fn score(mean: f64, half_width: f64, scale: Scale) -> Option<f64> {
    let size = scale.size();
    if !(half_width.is_finite() && mean.is_finite() && size.is_finite() && size > 0.0) {
        return None;
    }
    match scale {
        Scale::Points(p) => Some(half_width * 100.0 / p),
        Scale::Absolute(a) => Some(half_width / a),
        Scale::Relative(_) if mean == 0.0 => None,
        Scale::Relative(r) => Some(half_width / mean.abs() / r),
    }
}

/// About how many replications bring a result whose [`score`] is `score` after `n` down to
/// settled (a score of 1): the interval narrows with the square root of the replications.
/// Rounded up to a multiple of ten, and always more than `n`.
pub fn runs_needed(n: u64, score: f64) -> u64 {
    let wanted = (n as f64 * score * score).ceil().min(1.0e6) as u64;
    let wanted = wanted.div_ceil(10) * 10;
    wanted.max(n + 1)
}

/// What a sequential run makes of one result after some replications.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Its half-width is within the target; its [`score`].
    Settled(f64),
    /// Its half-width is wider than the target; its [`score`].
    Wide(f64),
    /// Every replication so far gave the same value, so its half-width is 0. Not settled: the
    /// replications made may simply not have met a rare event yet.
    NoSpread,
    /// A relative target on a mean at zero, or so near it that its interval takes in zero: no
    /// share of the mean can be met there. Its [`score`] where there is one.
    MeanNearZero(Option<f64>),
    /// Fewer than two replications had a value, so there is no interval yet.
    NoInterval,
    /// No replication had a value for it.
    NoValue,
}

impl Verdict {
    pub fn is_settled(self) -> bool {
        matches!(self, Verdict::Settled(_))
    }

    /// Its name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Settled(_) => "settled",
            Verdict::Wide(_) => "wide",
            Verdict::NoSpread => "no_spread",
            Verdict::MeanNearZero(_) => "mean_near_zero",
            Verdict::NoInterval => "no_interval",
            Verdict::NoValue => "no_value",
        }
    }

    /// Its [`score`], where it has one.
    pub fn score(self) -> Option<f64> {
        match self {
            Verdict::Settled(s) | Verdict::Wide(s) | Verdict::MeanNearZero(Some(s)) => Some(s),
            _ => None,
        }
    }
}

/// The verdict on a result over `n` replications with `mean`, 95% `half_width` (`None` or
/// infinite without an interval) and sample standard deviation `std_dev`, against `scale`.
/// Zero spread is looked at before the score, since a half-width of 0 scores 0.
pub fn verdict(
    n: u64,
    mean: Option<f64>,
    half_width: Option<f64>,
    std_dev: Option<f64>,
    scale: Scale,
) -> Verdict {
    let Some(mean) = mean.filter(|m| m.is_finite()) else {
        return Verdict::NoValue;
    };
    let Some(hw) = half_width.filter(|h| n >= 2 && h.is_finite()) else {
        return Verdict::NoInterval;
    };
    if std_dev.is_some_and(|s| s == 0.0) || hw == 0.0 {
        return Verdict::NoSpread;
    }
    let s = score(mean, hw, scale);
    if matches!(scale, Scale::Relative(_)) && (mean == 0.0 || mean.abs() <= hw) {
        return Verdict::MeanNearZero(s);
    }
    match s {
        Some(s) if s <= 1.0 => Verdict::Settled(s),
        Some(s) => Verdict::Wide(s),
        None => Verdict::NoInterval,
    }
}

/// Why a sequential run stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Every result it watches is settled.
    TargetMet,
    /// It reached its maximum with some result wider than its target.
    MaxReached,
    /// It reached its maximum with some result whose replications all gave the same value.
    NoSpread,
    /// It reached its maximum with some result on a relative target whose mean is at or near
    /// zero.
    MeanNearZero,
}

impl Reason {
    /// Its name on the wire.
    pub fn name(self) -> &'static str {
        match self {
            Reason::TargetMet => "target_met",
            Reason::MaxReached => "max_reached",
            Reason::NoSpread => "no_spread",
            Reason::MeanNearZero => "mean_near_zero",
        }
    }
}

/// What a sequential run does after `n` replications.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Run replications `n` up to (not including) this count, then judge again.
    Continue(u64),
    /// Stop here, for this reason.
    Stop(Reason),
}

/// The next step of a sequential run after `n` replications whose watched results got
/// `verdicts`, with at most `max` replications in all.
///
/// It stops when every verdict is settled (a run watching nothing stops at once), or at `max`:
/// then the reason is [`Reason::NoSpread`] if some result had no spread, else
/// [`Reason::MeanNearZero`] if some mean was at or near zero, else [`Reason::MaxReached`].
/// Otherwise the next batch runs up to [`runs_needed`] for the widest score among the results
/// not settled; where some result not settled has no score to estimate from (no spread, no
/// interval, no value, a mean of zero), at least twice `n`. Always more than `n`, at most `max`.
pub fn next_count(n: u64, max: u64, verdicts: &[Verdict]) -> Step {
    let open: Vec<Verdict> = verdicts
        .iter()
        .copied()
        .filter(|v| !v.is_settled())
        .collect();
    if open.is_empty() {
        return Step::Stop(Reason::TargetMet);
    }
    if n >= max {
        let reason = if open.contains(&Verdict::NoSpread) {
            Reason::NoSpread
        } else if open.iter().any(|v| matches!(v, Verdict::MeanNearZero(_))) {
            Reason::MeanNearZero
        } else {
            Reason::MaxReached
        };
        return Step::Stop(reason);
    }
    let widest = open
        .iter()
        .filter_map(|v| v.score())
        .fold(None, |w: Option<f64>, s| Some(w.map_or(s, |w| w.max(s))));
    let estimated = widest.map_or(n + 1, |s| runs_needed(n.max(1), s));
    let unknown = open.iter().any(|v| v.score().is_none());
    let doubled = if unknown { n.max(1) * 2 } else { 0 };
    Step::Continue(estimated.max(doubled).max(n + 1).min(max))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Summary;

    fn judge(values: &[f64], scale: Scale) -> Verdict {
        let s = Summary::of(values);
        verdict(
            s.map_or(0, |s| s.n),
            s.map(|s| s.mean),
            s.map(|s| s.half_width),
            s.map(|s| s.std_dev),
            scale,
        )
    }

    /// The window's own cases: a share in points, a time against its mean.
    #[test]
    fn scores() {
        assert!(score(0.017, 0.006, Scale::Points(SETTLED_POINTS)).is_some_and(|s| s <= 1.0));
        assert!(score(0.92, 0.03, Scale::Points(SETTLED_POINTS)).is_some_and(|s| s > 2.0));
        assert!(
            score(38.0, 12.16, Scale::Relative(SETTLED)).is_some_and(|s| (s - 6.4).abs() < 1e-9)
        );
        assert!(score(0.0, 0.0, Scale::Relative(SETTLED)).is_none());
        assert!(score(1.0, f64::INFINITY, Scale::Relative(SETTLED)).is_none());
        assert_eq!(score(5.0, 0.5, Scale::Absolute(0.25)), Some(2.0));
        assert!(score(5.0, 0.5, Scale::Absolute(0.0)).is_none());
    }

    #[test]
    fn runs_needed_grows_with_the_square_of_the_score() {
        assert_eq!(runs_needed(10, 2.0), 40);
        assert_eq!(runs_needed(10, 1.05), 20);
        assert_eq!(runs_needed(10, 0.5), 11);
    }

    /// Dot's fixture: each run returns 1, except during a rare disruption (1% of runs), when it
    /// returns 101. With the first runs all 1, the sample spread is zero, so a plain Student-t
    /// rule sees a half-width of 0 and stops with 1 ± 0. This one must not.
    #[test]
    fn zero_spread_never_settles_a_result_on_its_own() {
        let scale = Scale::Relative(SETTLED);
        let ones = vec![1.0; 200];
        // After the first 10, all 1: no spread, so not settled, and the run goes on.
        assert_eq!(judge(&ones[..10], scale), Verdict::NoSpread);
        assert_eq!(
            next_count(10, 200, &[judge(&ones[..10], scale)]),
            Step::Continue(20)
        );
        // Still all 1 at every stage: it runs to the maximum and says why.
        let mut n = 10;
        let mut stages = vec![n];
        let reason = loop {
            match next_count(n, 200, &[judge(&ones[..n as usize], scale)]) {
                Step::Continue(next) => {
                    assert!(next > n && next <= 200);
                    n = next;
                    stages.push(n);
                }
                Step::Stop(reason) => break reason,
            }
        };
        assert_eq!(n, 200);
        assert_eq!(reason, Reason::NoSpread);
        assert_eq!(stages, vec![10, 20, 40, 80, 160, 200]);
    }

    /// With the disruption arriving in run 31 (the 101 at index 30), the run never reports the
    /// target met with a mean of 1 and a half-width of 0: once the 101 is seen, the spread is
    /// wide, and it runs on to the maximum.
    #[test]
    fn a_rare_disruption_is_not_hidden_by_the_first_runs() {
        let scale = Scale::Relative(SETTLED);
        let mut values = vec![1.0; 200];
        values[30] = 101.0;
        let mut n = 10;
        let (reason, last) = loop {
            let v = judge(&values[..n as usize], scale);
            match next_count(n, 200, &[v]) {
                Step::Continue(next) => n = next,
                Step::Stop(reason) => break (reason, v),
            }
        };
        assert_ne!(reason, Reason::TargetMet);
        assert_eq!(n, 200);
        assert_eq!(reason, Reason::MaxReached);
        assert!(matches!(last, Verdict::Wide(_)), "{last:?}");
        let s = Summary::of(&values);
        assert!(s.is_some_and(|s| (s.mean - 1.5).abs() < 1e-12 && s.half_width > 0.0));
    }

    /// The disruption itself at 1%, in runs from a pseudo-random sequence (a 64-bit xorshift,
    /// since this crate uses no other): whatever the sequence, a run that says the target was
    /// met has a spread.
    #[test]
    fn a_met_target_always_has_a_spread() {
        for seed in 1..=50u64 {
            let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
            let values: Vec<f64> = (0..200)
                .map(|_| {
                    x ^= x << 13;
                    x ^= x >> 7;
                    x ^= x << 17;
                    if x % 100 == 0 { 101.0 } else { 1.0 }
                })
                .collect();
            let mut n = 10;
            loop {
                let v = judge(&values[..n as usize], Scale::Relative(SETTLED));
                match next_count(n, 200, &[v]) {
                    Step::Continue(next) => n = next,
                    Step::Stop(Reason::TargetMet) => {
                        let s = Summary::of(&values[..n as usize]);
                        assert!(s.is_some_and(|s| s.std_dev > 0.0 && s.half_width > 0.0));
                        break;
                    }
                    Step::Stop(_) => break,
                }
            }
        }
    }

    #[test]
    fn a_mean_near_zero_never_meets_a_relative_target() {
        // Values around zero: the interval takes in zero.
        let values = [-1.0, 1.0, -0.5, 0.6, 0.1, -0.2, 0.3, -0.4, 0.05, 0.0];
        let v = judge(&values, Scale::Relative(SETTLED));
        assert!(matches!(v, Verdict::MeanNearZero(_)), "{v:?}");
        assert_eq!(next_count(10, 10, &[v]), Step::Stop(Reason::MeanNearZero));
        assert!(matches!(next_count(10, 200, &[v]), Step::Continue(n) if n > 10));
        // A mean of exactly zero with a spread: no score at all, and the run doubles.
        let zero = [-1.0, 1.0];
        let v = judge(&zero, Scale::Relative(SETTLED));
        assert_eq!(v, Verdict::MeanNearZero(None));
        assert_eq!(next_count(2, 200, &[v]), Step::Continue(4));
        // The same values against an absolute half-width can be met.
        let v = judge(&values, Scale::Absolute(1.0));
        assert!(v.is_settled(), "{v:?}");
        assert_eq!(next_count(10, 200, &[v]), Step::Stop(Reason::TargetMet));
    }

    #[test]
    fn no_spread_outranks_a_mean_near_zero_at_the_maximum() {
        let near = Verdict::MeanNearZero(Some(30.0));
        assert_eq!(
            next_count(50, 50, &[Verdict::Wide(3.0), near, Verdict::NoSpread]),
            Step::Stop(Reason::NoSpread)
        );
        assert_eq!(
            next_count(50, 50, &[Verdict::Wide(3.0), near]),
            Step::Stop(Reason::MeanNearZero)
        );
        assert_eq!(
            next_count(50, 50, &[Verdict::Wide(3.0), Verdict::Settled(0.5)]),
            Step::Stop(Reason::MaxReached)
        );
        // Settled everywhere stops before the maximum, and so does watching nothing.
        assert_eq!(
            next_count(10, 50, &[Verdict::Settled(0.9)]),
            Step::Stop(Reason::TargetMet)
        );
        assert_eq!(next_count(10, 50, &[]), Step::Stop(Reason::TargetMet));
    }

    #[test]
    fn batches_come_from_the_widest_score_and_stay_within_the_maximum() {
        // Widest score 2 after 10: about 40 needed.
        assert_eq!(
            next_count(10, 200, &[Verdict::Wide(2.0), Verdict::Wide(1.2)]),
            Step::Continue(40)
        );
        // Capped at the maximum.
        assert_eq!(
            next_count(10, 30, &[Verdict::Wide(2.0)]),
            Step::Continue(30)
        );
        // A result with no score to estimate from doubles the count at least.
        assert_eq!(
            next_count(10, 200, &[Verdict::Wide(1.1), Verdict::NoInterval]),
            Step::Continue(20)
        );
        // One replication: no interval yet.
        assert_eq!(
            next_count(1, 200, &[Verdict::NoInterval]),
            Step::Continue(2)
        );
        assert_eq!(judge(&[3.0], Scale::Relative(SETTLED)), Verdict::NoInterval);
        assert_eq!(judge(&[], Scale::Relative(SETTLED)), Verdict::NoValue);
    }
}
