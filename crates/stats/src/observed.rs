//! Checking a simulated result against real data (black-box validation: A. M. Law, *Simulation
//! Modeling and Analysis*, 5th ed., McGraw-Hill, 2015, section 5.6.2; Sargent, "Verification and
//! validation of simulation models", *Journal of Simulation* 7, 2013).
//!
//! A result of the real system is either an observed value, with a 95% half-width of its own
//! where its data give one, or a bound the result must meet. The simulated result is the mean of
//! the replications with its 95% Student-t half-width.
//!
//! - An observed value is checked by the interval of the difference (simulated mean minus
//!   observed): the two agree when it contains 0. With the half-width `hs` of the simulated mean
//!   and `ho` of the observed value the difference's half-width is `sqrt(hs² + ho²)`, the usual
//!   large-sample approximation for two independent estimates (the two means' intervals are
//!   combined as their variances would be, not added, and not given Welch's degrees of
//!   freedom); without an `ho` it is `hs`.
//! - A bound is decided by the whole simulated interval, never by the mean alone: it passes when
//!   every value in the interval meets it, fails when every value violates it, and is not yet
//!   decided when the interval straddles it (more replications would narrow the interval).
//! - Fewer than 2 replications give no interval, so nothing can be decided.

/// What a simulated result is checked against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Check {
    /// The value seen in the real system, with the half-width of its own 95% interval where
    /// known.
    Value { value: f64, half_width: Option<f64> },
    /// The result should be at most this.
    AtMost(f64),
    /// The result should be at least this.
    AtLeast(f64),
}

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// An observed value: the interval of the difference contains 0.
    Agrees,
    /// An observed value: the interval of the difference excludes 0.
    Differs,
    /// A bound: the whole interval meets it.
    Passes,
    /// A bound: the whole interval violates it.
    Fails,
    /// No interval to judge by (fewer than 2 replications), or a bound the interval straddles:
    /// more replications would decide it.
    CantTell,
}

impl Outcome {
    /// A stable name for files, the command line and agents.
    pub fn name(self) -> &'static str {
        match self {
            Outcome::Agrees => "agrees",
            Outcome::Differs => "differs",
            Outcome::Passes => "passes",
            Outcome::Fails => "fails",
            Outcome::CantTell => "cant_tell",
        }
    }

    /// Whether the check found the result wrong: the observed value differs, or the bound is
    /// violated.
    pub fn is_bad(self) -> bool {
        matches!(self, Outcome::Differs | Outcome::Fails)
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
/// An infinite or negative half-width, or fewer than 2 replications, give no interval.
pub fn judge(check: Check, mean: f64, half_width: f64, n: u64) -> Verdict {
    let has_interval = n >= 2 && mean.is_finite() && half_width.is_finite() && half_width >= 0.0;
    match check {
        Check::Value {
            value,
            half_width: observed,
        } => {
            let difference = (mean.is_finite() && value.is_finite()).then(|| mean - value);
            if !has_interval || !value.is_finite() {
                return Verdict {
                    outcome: Outcome::CantTell,
                    difference,
                    half_width: None,
                };
            }
            // A missing or unusable observed half-width counts as 0: the observed value is taken
            // as exact.
            let ho = observed
                .filter(|h| h.is_finite() && *h >= 0.0)
                .unwrap_or(0.0);
            let combined = (half_width * half_width + ho * ho).sqrt();
            let d = mean - value;
            Verdict {
                outcome: if d.abs() <= combined {
                    Outcome::Agrees
                } else {
                    Outcome::Differs
                },
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
                return none(Outcome::CantTell);
            }
            let (low, high) = (mean - half_width, mean + half_width);
            let outcome = match check {
                Check::AtMost(_) if high <= bound => Outcome::Passes,
                Check::AtMost(_) if low > bound => Outcome::Fails,
                Check::AtLeast(_) if low >= bound => Outcome::Passes,
                Check::AtLeast(_) if high < bound => Outcome::Fails,
                _ => Outcome::CantTell,
            };
            none(outcome)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value(v: f64, h: Option<f64>) -> Check {
        Check::Value {
            value: v,
            half_width: h,
        }
    }

    #[test]
    fn an_observed_value_agrees_when_the_interval_of_the_difference_holds_0() {
        // 10 ± 1 against an exact 10.8: the difference, -0.8, is inside ± 1.
        let v = judge(value(10.8, None), 10.0, 1.0, 10);
        assert_eq!(v.outcome, Outcome::Agrees);
        assert!(v.difference.is_some_and(|d| (d + 0.8).abs() < 1e-12));
        assert_eq!(v.half_width, Some(1.0));
        // Against 11.2 it is outside.
        let v = judge(value(11.2, None), 10.0, 1.0, 10);
        assert_eq!(v.outcome, Outcome::Differs);
        // Exactly on the edge agrees: the interval is closed.
        assert_eq!(
            judge(value(11.0, None), 10.0, 1.0, 10).outcome,
            Outcome::Agrees
        );
    }

    #[test]
    fn the_two_half_widths_combine_as_the_root_of_the_sum_of_squares() {
        // sqrt(1.5² + 1.5²) = 2.1213: a difference of 2 is inside, though it is outside either
        // half-width alone.
        let v = judge(value(12.0, Some(1.5)), 10.0, 1.5, 10);
        assert_eq!(v.outcome, Outcome::Agrees);
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
        assert_eq!(judge(at_most, 1.8, 0.4, 10).outcome, Outcome::CantTell);
        // The mean is over the bound but the interval straddles it: not decided.
        assert_eq!(judge(at_most, 2.1, 0.4, 10).outcome, Outcome::CantTell);
        // 2.5 ± 0.4 is within 2.1 to 2.9: fails.
        assert_eq!(judge(at_most, 2.5, 0.4, 10).outcome, Outcome::Fails);
        // The top of the interval on the bound meets it; the bottom on it does not violate it
        // (values that are exact in binary, so the edges are exact).
        assert_eq!(judge(at_most, 1.5, 0.5, 10).outcome, Outcome::Passes);
        assert_eq!(judge(at_most, 2.5, 0.5, 10).outcome, Outcome::CantTell);
    }

    #[test]
    fn a_lower_bound_is_decided_by_the_whole_interval() {
        let at_least = Check::AtLeast(400.0);
        assert_eq!(judge(at_least, 450.0, 20.0, 10).outcome, Outcome::Passes);
        assert_eq!(judge(at_least, 410.0, 20.0, 10).outcome, Outcome::CantTell);
        assert_eq!(judge(at_least, 390.0, 20.0, 10).outcome, Outcome::CantTell);
        assert_eq!(judge(at_least, 350.0, 20.0, 10).outcome, Outcome::Fails);
        assert_eq!(judge(at_least, 420.0, 20.0, 10).outcome, Outcome::Passes);
        assert_eq!(judge(at_least, 380.0, 20.0, 10).outcome, Outcome::CantTell);
    }

    #[test]
    fn a_bound_has_no_difference() {
        let v = judge(Check::AtMost(2.0), 1.5, 0.4, 10);
        assert_eq!((v.difference, v.half_width), (None, None));
    }

    #[test]
    fn fewer_than_two_replications_decide_nothing() {
        for check in [value(10.0, None), Check::AtMost(100.0), Check::AtLeast(0.0)] {
            // One replication: an infinite half-width (or zero, whichever the caller has).
            for hw in [f64::INFINITY, 0.0] {
                assert_eq!(judge(check, 10.0, hw, 1).outcome, Outcome::CantTell);
            }
            assert_eq!(judge(check, 10.0, 0.0, 0).outcome, Outcome::CantTell);
            assert_eq!(judge(check, 10.0, f64::NAN, 10).outcome, Outcome::CantTell);
            assert_eq!(judge(check, f64::NAN, 1.0, 10).outcome, Outcome::CantTell);
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
            Outcome::Agrees
        );
        assert_eq!(
            judge(value(101.0, None), 100.0, 0.0, 10).outcome,
            Outcome::Differs
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
    fn only_a_differing_value_or_a_violated_bound_is_bad() {
        assert!(Outcome::Differs.is_bad() && Outcome::Fails.is_bad());
        assert!(
            !Outcome::Agrees.is_bad() && !Outcome::Passes.is_bad() && !Outcome::CantTell.is_bad()
        );
        let names: Vec<&str> = [
            Outcome::Agrees,
            Outcome::Differs,
            Outcome::Passes,
            Outcome::Fails,
            Outcome::CantTell,
        ]
        .into_iter()
        .map(Outcome::name)
        .collect();
        assert_eq!(names, ["agrees", "differs", "passes", "fails", "cant_tell"]);
    }
}
