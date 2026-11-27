//! Checking a simulated result against real data (black-box validation: A. M. Law, *Simulation
//! Modeling and Analysis*, 5th ed., McGraw-Hill, 2015, section 5.6.2; Sargent, "Verification and
//! validation of simulation models", *Journal of Simulation* 7, 2013).
//!
//! A result of the real system is either an observed value, with a 95% half-width of its own
//! where its data give one, or a bound the result must meet. The simulated result is the mean of
//! the replications with its 95% Student-t half-width.
//!
//! - An observed value is checked by the 95% interval of the difference (simulated mean minus
//!   observed). With the half-width `hs` of the simulated mean and `ho` of the observed value its
//!   half-width is `hypot(hs, ho)`, the usual large-sample approximation for two independent
//!   estimates (variances combined, not half-widths added, and no Welch degrees of freedom);
//!   without an `ho` it is `hs`.
//!   - Without a tolerance: an interval that excludes 0 is a difference (`Differs`); one that
//!     holds 0 is only "no difference detected", never agreement: a wide interval holds 0 whatever
//!     the truth, so failing to detect a difference is not evidence there is none (Sargent 2013).
//!   - With a tolerance `t`, set in advance by the modeller, the check is an equivalence test: the
//!     whole interval inside [-t, +t] is `Agrees`, the whole interval outside it (entirely above
//!     +t or below -t) is `Differs`, anything else `Inconclusive`. This is the two one-sided
//!     tests rule (Schuirmann 1987) with the 95% interval, which is conservative against the 90%
//!     interval of the exact test.
//! - [`judge_welch`]: where the observed value comes from a known number m of independent data
//!   sets (its half-width is then their 95% Student-t half-width), the difference's interval is
//!   Welch's (Law, section 5.6.2, by section 10.2.2): each side's variance of the mean recovered
//!   from its half-width and count, (h/t)², the Welch–Satterthwaite degrees of freedom rounded
//!   down, and the t value at the run's confidence level. Without the count, [`judge`]'s rule.
//!   A real system and the model's runs share no random numbers, so they are never paired.
//! - A bound is decided by the whole simulated interval, never by the mean alone: it passes when
//!   every value in the interval meets it, fails when every value violates it, and is
//!   inconclusive when the interval straddles it (more replications would narrow the interval).
//! - Fewer than 2 replications give no interval, so nothing can be decided: inconclusive.

/// What a simulated result is checked against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    /// The value seen in the real system, with the half-width of its own 95% interval where
    /// known, and a tolerance (above 0) within which the model counts as agreeing, where the
    /// modeller has set one.
    Value {
        value: f64,
        half_width: Option<f64>,
        tolerance: Option<f64>,
    },
    /// The result should be at most this.
    AtMost(f64),
    /// The result should be at least this.
    AtLeast(f64),
}

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An observed value with a tolerance: the whole interval of the difference is inside it.
    Agrees,
    /// An observed value: the interval of the difference excludes 0 (with a tolerance: lies
    /// wholly outside it).
    Differs,
    /// An observed value with no tolerance: the interval of the difference holds 0. Not evidence
    /// of agreement.
    NoDifferenceDetected,
    /// A bound: the whole interval meets it.
    Passes,
    /// A bound: the whole interval violates it.
    Fails,
    /// No interval to judge by (fewer than 2 replications), a bound the interval straddles, or a
    /// difference interval that is neither inside nor outside the tolerance: more replications
    /// would narrow it.
    Inconclusive,
}

impl Outcome {
    /// A stable name for files, the command line and agents.
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Agrees => "agrees",
            Outcome::Differs => "differs",
            Outcome::NoDifferenceDetected => "no_difference_detected",
            Outcome::Passes => "passes",
            Outcome::Fails => "fails",
            Outcome::Inconclusive => "inconclusive",
        }
    }

    /// Whether the check found the result wrong: the observed value differs, or the bound is
    /// violated.
    pub fn is_bad(self) -> bool {
        matches!(self, Outcome::Differs | Outcome::Fails)
    }

    /// Where the check sorts in the results: those found wrong first (0), then those not decided
    /// or only without a detected difference (1), then those found right (2).
    pub fn rank(self) -> u8 {
        match self {
            Outcome::Differs | Outcome::Fails => 0,
            Outcome::Inconclusive | Outcome::NoDifferenceDetected => 1,
            Outcome::Agrees | Outcome::Passes => 2,
        }
    }
}

/// A check's outcome with the difference it was decided by.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    pub outcome: Outcome,
    /// For an observed value: the simulated mean minus the observed value. `None` for a bound,
    /// and where there is no simulated mean.
    pub difference: Option<f64>,
    /// For an observed value: the half-width of the difference's 95% interval. `None` for a
    /// bound, and where there is no interval.
    pub half_width: Option<f64>,
}

/// Judges the simulated `mean` with its 95% `half_width` over `n` replications against `check`.
/// An infinite or negative half-width, or fewer than 2 replications, give no interval. An
/// unusable tolerance (not finite, or not above 0) counts as none.
pub fn judge(check: Check, mean: f64, half_width: f64, n: u64) -> Verdict {
    let has_interval = n >= 2 && mean.is_finite() && half_width.is_finite() && half_width >= 0.0;
    match check {
        Check::Value {
            value,
            half_width: observed,
            tolerance,
        } => {
            let difference = (mean.is_finite() && value.is_finite()).then_some(mean - value);
            if !has_interval || !value.is_finite() {
                return Verdict {
                    outcome: Outcome::Inconclusive,
                    difference,
                    half_width: None,
                };
            }
            // A missing or unusable observed half-width counts as 0: the observed value is taken
            // as exact.
            let ho = observed
                .filter(|h| h.is_finite() && *h >= 0.0)
                .unwrap_or(0.0);
            // Scaled, so neither a tiny nor a huge width is lost to underflow or overflow.
            let combined = half_width.hypot(ho);
            let d = mean - value;
            Verdict {
                outcome: value_outcome(d, combined, tolerance),
                difference,
                half_width: Some(combined),
            }
        }
        Check::AtMost(bound) | Check::AtLeast(bound) => {
            let none = |outcome| Verdict {
                outcome,
                difference: None,
                half_width: None,
            };
            if !has_interval || !bound.is_finite() {
                return none(Outcome::Inconclusive);
            }
            none(bound_outcome(
                matches!(check, Check::AtMost(_)),
                mean,
                half_width,
                bound,
            ))
        }
    }
}

/// The half-width of the interval of (simulated mean − observed value) by Welch's method, and its
/// degrees of freedom: the simulated half-width `hs` over `n` replications at confidence `level`,
/// and the observed `ho`, a 95% half-width from `sets` independent data sets. Each side's
/// variance of the mean is (h/t)² with its own t (t_{n−1} at `level`, t_{sets−1} at 95%), and the
/// interval is at `level`. `None` with fewer than 2 on either side, or a half-width that is not
/// finite or below 0.
pub fn welch_half_width(hs: f64, n: u64, level: f64, ho: f64, sets: u64) -> Option<(f64, u64)> {
    if n < 2 || sets < 2 || !(hs.is_finite() && hs >= 0.0 && ho.is_finite() && ho >= 0.0) {
        return None;
    }
    let ts = crate::critical::t_two_sided(level, n - 1)?;
    let to = crate::critical::t_two_sided(crate::critical::DEFAULT_LEVEL, sets - 1)?;
    let (us, uo) = ((hs / ts) * (hs / ts), (ho / to) * (ho / to));
    let se2 = us + uo;
    let df = if se2 > 0.0 {
        let f = se2 * se2 / (us * us / (n - 1) as f64 + uo * uo / (sets - 1) as f64);
        (f.floor() as u64).max(1)
    } else {
        n + sets - 2
    };
    let t = crate::critical::t_two_sided(level, df)?;
    Some((t * se2.sqrt(), df))
}

/// [`judge`] for an observed value from `sets` independent data sets, its half-width their 95%
/// interval: the difference's interval by Welch's method ([`welch_half_width`]), with the
/// simulated half-width at confidence `level`. With no usable count or no observed half-width,
/// or for a bound, exactly [`judge`]. Also gives the Welch degrees of freedom where they were
/// used.
pub fn judge_welch(
    check: Check,
    mean: f64,
    half_width: f64,
    n: u64,
    level: f64,
    sets: Option<u64>,
) -> (Verdict, Option<u64>) {
    let plain = judge(check, mean, half_width, n);
    let Check::Value {
        value,
        half_width: Some(ho),
        tolerance,
    } = check
    else {
        return (plain, None);
    };
    if !(mean.is_finite() && value.is_finite()) {
        return (plain, None);
    }
    let Some((combined, df)) = sets.and_then(|m| welch_half_width(half_width, n, level, ho, m))
    else {
        return (plain, None);
    };
    let d = mean - value;
    (
        Verdict {
            outcome: value_outcome(d, combined, tolerance),
            difference: Some(d),
            half_width: Some(combined),
        },
        Some(df),
    )
}

/// The outcome for an observed value: the difference `d` with the half-width `combined` of its
/// interval, against the tolerance, if there is a usable one.
fn value_outcome(d: f64, combined: f64, tolerance: Option<f64>) -> Outcome {
    let (low, high) = (d - combined, d + combined);
    match tolerance.filter(|t| t.is_finite() && *t > 0.0) {
        Some(t) if low >= -t && high <= t => Outcome::Agrees,
        Some(t) if low > t || high < -t => Outcome::Differs,
        Some(_) => Outcome::Inconclusive,
        None if d.abs() > combined => Outcome::Differs,
        None => Outcome::NoDifferenceDetected,
    }
}

/// The outcome for a bound (`at_most`, or at least) the interval `mean` +- `half_width` is
/// judged against as a whole.
fn bound_outcome(at_most: bool, mean: f64, half_width: f64, bound: f64) -> Outcome {
    let (low, high) = (mean - half_width, mean + half_width);
    if at_most {
        if high <= bound {
            Outcome::Passes
        } else if low > bound {
            Outcome::Fails
        } else {
            Outcome::Inconclusive
        }
    } else if low >= bound {
        Outcome::Passes
    } else if high < bound {
        Outcome::Fails
    } else {
        Outcome::Inconclusive
    }
}

/// Machine-checked properties (Kani, `cargo kani -p stochcraft-stats`; see docs/developers.md).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// An observed value agrees only inside its tolerance and differs only outside it, so the
    /// two verdicts cannot both describe one interval. With no usable tolerance it never agrees.
    #[kani::proof]
    fn a_value_agrees_only_inside_its_tolerance_and_differs_only_outside() {
        let (d, combined): (f64, f64) = (kani::any(), kani::any());
        kani::assume(d.is_finite() && combined.is_finite() && combined >= 0.0);
        let tolerance: Option<f64> = if kani::any() { Some(kani::any()) } else { None };
        let usable = tolerance.filter(|t| t.is_finite() && *t > 0.0);
        match value_outcome(d, combined, tolerance) {
            Outcome::Agrees => {
                let t = usable.unwrap_or(f64::NAN);
                assert!(d >= -t && d <= t);
            }
            Outcome::Differs => match usable {
                Some(t) => assert!(d > t || d < -t),
                None => assert!(d.abs() > combined),
            },
            Outcome::NoDifferenceDetected => assert!(usable.is_none() && d.abs() <= combined),
            Outcome::Inconclusive => assert!(usable.is_some()),
            Outcome::Passes | Outcome::Fails => panic!("a value is never judged as a bound"),
        }
    }

    /// A bound passes only if the mean itself meets it, and fails only if the mean violates it.
    #[kani::proof]
    fn a_bound_never_contradicts_the_point_estimate() {
        let (mean, half_width, bound): (f64, f64, f64) = (kani::any(), kani::any(), kani::any());
        kani::assume(mean.is_finite() && half_width.is_finite() && half_width >= 0.0);
        kani::assume(bound.is_finite());
        let at_most: bool = kani::any();
        match bound_outcome(at_most, mean, half_width, bound) {
            Outcome::Passes => assert!(if at_most {
                mean <= bound
            } else {
                mean >= bound
            }),
            Outcome::Fails => assert!(if at_most { mean > bound } else { mean < bound }),
            Outcome::Inconclusive => {}
            _ => panic!("a bound is only passed, failed or inconclusive"),
        }
    }

    /// With fewer than two replications nothing is decided, whatever the check or the numbers.
    #[kani::proof]
    fn fewer_than_two_replications_are_inconclusive() {
        let n: u64 = kani::any();
        kani::assume(n < 2);
        let (mean, half_width, x): (f64, f64, f64) = (kani::any(), kani::any(), kani::any());
        // Kani flags any NaN-producing operation, and `judge` subtracts before it looks at
        // the inputs (the result is discarded as NaN is not finite), so keep them finite.
        kani::assume(mean.is_finite() && half_width.is_finite() && x.is_finite());
        let check = match kani::any::<u8>() % 3 {
            0 => Check::AtMost(x),
            1 => Check::AtLeast(x),
            _ => Check::Value {
                value: x,
                half_width: None,
                tolerance: None,
            },
        };
        assert_eq!(
            judge(check, mean, half_width, n).outcome,
            Outcome::Inconclusive
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(v: f64, h: Option<f64>) -> Check {
        Check::Value {
            value: v,
            half_width: h,
            tolerance: None,
        }
    }

    fn within(v: f64, tolerance: f64) -> Check {
        Check::Value {
            value: v,
            half_width: None,
            tolerance: Some(tolerance),
        }
    }

    #[test]
    fn without_a_tolerance_an_interval_holding_0_is_not_agreement() {
        // Observed 10: 12 ± 0.2 excludes 0 and differs; 12 ± 5 holds 0 (the interval allows
        // -3 to +7), which is no evidence of a match, so it is never "agrees".
        let v = judge(value(10.0, None), 12.0, 0.2, 10);
        assert_eq!(v.outcome, Outcome::Differs);
        let v = judge(value(10.0, None), 12.0, 5.0, 10);
        assert_eq!(v.outcome, Outcome::NoDifferenceDetected);
        // 10.8 against 10 ± 1 holds 0: no difference detected. 11.2 does not.
        let v = judge(value(10.8, None), 10.0, 1.0, 10);
        assert_eq!(v.outcome, Outcome::NoDifferenceDetected);
        assert!(v.difference.is_some_and(|d| (d + 0.8).abs() < 1e-12));
        assert_eq!(v.half_width, Some(1.0));
        assert_eq!(
            judge(value(11.2, None), 10.0, 1.0, 10).outcome,
            Outcome::Differs
        );
        // Exactly on the edge holds 0: the interval is closed.
        assert_eq!(
            judge(value(11.0, None), 10.0, 1.0, 10).outcome,
            Outcome::NoDifferenceDetected
        );
    }

    #[test]
    fn a_tolerance_makes_it_an_equivalence_test() {
        // Observed 10, tolerance 1, interval of the difference d ± hs.
        let c = within(10.0, 1.0);
        // 10.2 ± 0.3: -0.1 to 0.5 for the difference, inside ± 1.
        assert_eq!(judge(c, 10.2, 0.3, 10).outcome, Outcome::Agrees);
        // 12 ± 0.5: 1.5 to 2.5, wholly above +1.
        assert_eq!(judge(c, 12.0, 0.5, 10).outcome, Outcome::Differs);
        // 8 ± 0.5: wholly below -1.
        assert_eq!(judge(c, 8.0, 0.5, 10).outcome, Outcome::Differs);
        // 10.5 ± 1: -0.5 to 1.5 straddles +1.
        assert_eq!(judge(c, 10.5, 1.0, 10).outcome, Outcome::Inconclusive);
        // A wide interval around the observed value is inconclusive, not agreement.
        assert_eq!(judge(c, 10.0, 5.0, 10).outcome, Outcome::Inconclusive);
        // The edges: wholly inside includes the edge; wholly outside excludes it.
        assert_eq!(judge(c, 10.5, 0.5, 10).outcome, Outcome::Agrees);
        assert_eq!(judge(c, 11.5, 0.5, 10).outcome, Outcome::Inconclusive);
        assert_eq!(judge(c, 11.75, 0.5, 10).outcome, Outcome::Differs);
        // An unusable tolerance counts as none.
        for t in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                judge(within(10.0, t), 12.0, 5.0, 10).outcome,
                Outcome::NoDifferenceDetected,
                "{t}"
            );
        }
    }

    #[test]
    fn tiny_and_huge_half_widths_are_combined_without_underflow_or_overflow() {
        // 1e-200 ± 2e-200 against an exact 0: squaring 2e-200 would give 0, and "differs".
        assert_eq!(
            judge(value(1e-200, Some(2e-200)), 0.0, 0.0, 10).outcome,
            Outcome::NoDifferenceDetected
        );
        // Squaring 1e200 would give infinity, and no difference whatever the difference.
        let v = judge(value(0.0, Some(1e200)), 3e200, 1e200, 10);
        assert_eq!(v.outcome, Outcome::Differs);
        assert!(v.half_width.is_some_and(f64::is_finite));
    }

    #[test]
    fn the_two_half_widths_combine_as_the_root_of_the_sum_of_squares() {
        // sqrt(1.5² + 1.5²) = 2.1213: a difference of 2 is inside, though it is outside either
        // half-width alone.
        let v = judge(value(12.0, Some(1.5)), 10.0, 1.5, 10);
        assert_eq!(v.outcome, Outcome::NoDifferenceDetected);
        assert!(
            v.half_width
                .is_some_and(|h| (h - 4.5_f64.sqrt()).abs() < 1e-12)
        );
        assert_eq!(
            judge(value(12.0, None), 10.0, 1.5, 10).outcome,
            Outcome::Differs
        );
        // Half-widths 1 and 1.5: the root is 1.803 where the sum would be 2.5, so a difference of
        // 2.2 is outside, though adding the half-widths would put it inside.
        let v = judge(value(12.2, Some(1.5)), 10.0, 1.0, 10);
        assert_eq!(v.outcome, Outcome::Differs);
        assert!(
            v.half_width
                .is_some_and(|h| (h - 3.25_f64.sqrt()).abs() < 1e-12)
        );
    }

    #[test]
    fn an_unusable_observed_half_width_is_taken_as_exact() {
        for h in [Some(-1.0), Some(f64::NAN), Some(f64::INFINITY), None] {
            let v = judge(value(11.5, h), 10.0, 1.0, 10);
            assert_eq!(v.outcome, Outcome::Differs, "{h:?}");
            assert_eq!(v.half_width, Some(1.0), "{h:?}");
        }
    }

    #[test]
    fn an_upper_bound_is_decided_by_the_whole_interval() {
        let at_most = Check::AtMost(2.0);
        // 1.5 ± 0.4 is within 1.1 to 1.9: passes.
        assert_eq!(judge(at_most, 1.5, 0.4, 10).outcome, Outcome::Passes);
        // The mean is under the bound but the interval straddles it: not decided.
        assert_eq!(judge(at_most, 1.8, 0.4, 10).outcome, Outcome::Inconclusive);
        // The mean is over the bound but the interval straddles it: not decided.
        assert_eq!(judge(at_most, 2.1, 0.4, 10).outcome, Outcome::Inconclusive);
        // 2.5 ± 0.4 is within 2.1 to 2.9: fails.
        assert_eq!(judge(at_most, 2.5, 0.4, 10).outcome, Outcome::Fails);
        // The top of the interval on the bound meets it; the bottom on it does not violate it
        // (values that are exact in binary, so the edges are exact).
        assert_eq!(judge(at_most, 1.5, 0.5, 10).outcome, Outcome::Passes);
        assert_eq!(judge(at_most, 2.5, 0.5, 10).outcome, Outcome::Inconclusive);
    }

    #[test]
    fn a_lower_bound_is_decided_by_the_whole_interval() {
        let at_least = Check::AtLeast(400.0);
        assert_eq!(judge(at_least, 450.0, 20.0, 10).outcome, Outcome::Passes);
        assert_eq!(
            judge(at_least, 410.0, 20.0, 10).outcome,
            Outcome::Inconclusive
        );
        assert_eq!(
            judge(at_least, 390.0, 20.0, 10).outcome,
            Outcome::Inconclusive
        );
        assert_eq!(judge(at_least, 350.0, 20.0, 10).outcome, Outcome::Fails);
        assert_eq!(judge(at_least, 420.0, 20.0, 10).outcome, Outcome::Passes);
        assert_eq!(
            judge(at_least, 380.0, 20.0, 10).outcome,
            Outcome::Inconclusive
        );
    }

    /// Law, 5th ed., Example 5.42 (p. 275): 8 test missiles, mean miss distance 159.95 with
    /// S² = 355.75, against 15 simulated, 183.50 with S² = 545.71. Welch's 95% interval for
    /// the difference is 23.55 ± 18.97 (the book's is for system minus model, −23.55), with
    /// f̂ = 17.34: it excludes 0, so the model differs.
    #[test]
    fn welch_reproduces_the_missile_example() {
        let t = |df: u64| crate::critical::t_two_sided(0.95, df).unwrap_or(f64::NAN);
        let ho = t(7) * (355.75_f64 / 8.0).sqrt();
        let hs = t(14) * (545.71_f64 / 15.0).sqrt();
        let check = Check::Value {
            value: 159.95,
            half_width: Some(ho),
            tolerance: None,
        };
        let (v, df) = judge_welch(check, 183.50, hs, 15, 0.95, Some(8));
        assert_eq!(df, Some(17));
        assert_eq!(v.outcome, Outcome::Differs);
        assert!(v.difference.is_some_and(|d| (d - 23.55).abs() < 1e-9));
        assert!(
            v.half_width.is_some_and(|h| (h - 18.97).abs() < 5e-3),
            "{v:?}"
        );
        // Without the count: the hypot rule, exactly as `judge` gives it.
        let (plain, none) = judge_welch(check, 183.50, hs, 15, 0.95, None);
        assert_eq!((plain, none), (judge(check, 183.50, hs, 15), None));
        // A count of 1, no observed half-width, or a bound: `judge` too.
        assert_eq!(judge_welch(check, 183.50, hs, 15, 0.95, Some(1)).1, None);
        let bare = Check::Value {
            value: 159.95,
            half_width: None,
            tolerance: None,
        };
        assert_eq!(judge_welch(bare, 183.50, hs, 15, 0.95, Some(8)).1, None);
        assert_eq!(
            judge_welch(Check::AtMost(200.0), 183.50, hs, 15, 0.95, Some(8)),
            (judge(Check::AtMost(200.0), 183.50, hs, 15), None)
        );
    }

    #[test]
    fn a_bound_has_no_difference() {
        let v = judge(Check::AtMost(2.0), 1.5, 0.4, 10);
        assert_eq!((v.difference, v.half_width), (None, None));
    }

    #[test]
    fn fewer_than_two_replications_decide_nothing() {
        for check in [
            value(10.0, None),
            within(10.0, 1.0),
            Check::AtMost(100.0),
            Check::AtLeast(0.0),
        ] {
            // One replication: an infinite half-width (or zero, whichever the caller has).
            for hw in [f64::INFINITY, 0.0] {
                assert_eq!(judge(check, 10.0, hw, 1).outcome, Outcome::Inconclusive);
            }
            assert_eq!(judge(check, 10.0, 0.0, 0).outcome, Outcome::Inconclusive);
            assert_eq!(
                judge(check, 10.0, f64::NAN, 10).outcome,
                Outcome::Inconclusive
            );
            assert_eq!(
                judge(check, f64::NAN, 1.0, 10).outcome,
                Outcome::Inconclusive
            );
        }
        // The difference is still said, without an interval.
        let v = judge(value(8.0, None), 10.0, f64::INFINITY, 1);
        assert_eq!(v.difference, Some(2.0));
        assert_eq!(v.half_width, None);
    }

    #[test]
    fn no_spread_across_replications_is_an_exact_result() {
        // Every replication gave 100: the interval is a point.
        assert_eq!(
            judge(value(100.0, None), 100.0, 0.0, 10).outcome,
            Outcome::NoDifferenceDetected
        );
        assert_eq!(
            judge(value(101.0, None), 100.0, 0.0, 10).outcome,
            Outcome::Differs
        );
        assert_eq!(
            judge(within(101.0, 2.0), 100.0, 0.0, 10).outcome,
            Outcome::Agrees
        );
        assert_eq!(
            judge(Check::AtMost(100.0), 100.0, 0.0, 10).outcome,
            Outcome::Passes
        );
        assert_eq!(
            judge(Check::AtMost(99.0), 100.0, 0.0, 10).outcome,
            Outcome::Fails
        );
    }

    #[test]
    fn only_a_differing_value_or_a_violated_bound_is_bad_and_they_sort_first() {
        use Outcome::*;
        assert!(Differs.is_bad() && Fails.is_bad());
        for o in [Agrees, Passes, Inconclusive, NoDifferenceDetected] {
            assert!(!o.is_bad());
        }
        let ranks: Vec<u8> = [
            Differs,
            Fails,
            Inconclusive,
            NoDifferenceDetected,
            Agrees,
            Passes,
        ]
        .into_iter()
        .map(Outcome::rank)
        .collect();
        assert_eq!(ranks, [0, 0, 1, 1, 2, 2]);
        let names: Vec<&str> = [
            Agrees,
            Differs,
            NoDifferenceDetected,
            Passes,
            Fails,
            Inconclusive,
        ]
        .into_iter()
        .map(Outcome::name)
        .collect();
        assert_eq!(
            names,
            [
                "agrees",
                "differs",
                "no_difference_detected",
                "passes",
                "fails",
                "inconclusive"
            ]
        );
    }
}
