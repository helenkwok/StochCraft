//! Statistics for simulation output.
//!
//! - [`Tally`]: running count, mean, variance, min and max of observations (Welford's update,
//!   numerically stable in one pass), e.g. time in system per customer.
//! - [`TimeWeighted`]: the time-average of a quantity that changes at discrete instants, e.g.
//!   queue length or machine utilisation.
//! - [`Summary`]: the mean of independent replications with a 95% confidence interval from
//!   Student's t distribution.
//! - [`spread`]: the distribution of a result across replications: percentiles, the empirical
//!   cumulative distribution with its Dvoretzky–Kiefer–Wolfowitz band, and the probability of
//!   meeting a target with Wilson's interval.

pub mod spread;
pub use spread::{Ecdf, Proportion};

/// Running statistics over observations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tally {
    n: u64,
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
}

impl Tally {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an observation. Non-finite values are ignored.
    pub fn add(&mut self, x: f64) {
        if !x.is_finite() {
            return;
        }
        self.n += 1;
        if self.n == 1 {
            self.min = x;
            self.max = x;
        } else {
            self.min = self.min.min(x);
            self.max = self.max.max(x);
        }
        let delta = x - self.mean;
        self.mean += delta / self.n as f64;
        self.m2 += delta * (x - self.mean);
    }

    pub fn count(&self) -> u64 {
        self.n
    }

    /// The mean, or `None` with no observations.
    pub fn mean(&self) -> Option<f64> {
        (self.n > 0).then_some(self.mean)
    }

    /// The sample variance (n − 1 denominator), or `None` with fewer than two observations.
    pub fn variance(&self) -> Option<f64> {
        (self.n > 1).then(|| self.m2 / (self.n - 1) as f64)
    }

    pub fn std_dev(&self) -> Option<f64> {
        self.variance().map(f64::sqrt)
    }

    pub fn min(&self) -> Option<f64> {
        (self.n > 0).then_some(self.min)
    }

    pub fn max(&self) -> Option<f64> {
        (self.n > 0).then_some(self.max)
    }
}

/// The time-average of a piecewise-constant quantity.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeWeighted {
    start: f64,
    last_time: f64,
    value: f64,
    area: f64,
}

impl TimeWeighted {
    /// Starts recording `value` at time `start`.
    pub fn new(start: f64, value: f64) -> Self {
        Self {
            start,
            last_time: start,
            value,
            area: 0.0,
        }
    }

    /// The quantity changes to `value` at `time`. Times earlier than the last update are treated
    /// as the last update's time.
    pub fn set(&mut self, time: f64, value: f64) {
        let t = if time.is_finite() {
            time.max(self.last_time)
        } else {
            self.last_time
        };
        self.area += self.value * (t - self.last_time);
        self.last_time = t;
        if value.is_finite() {
            self.value = value;
        }
    }

    /// The current value.
    pub fn value(&self) -> f64 {
        self.value
    }

    /// The time-average from the start to `time`, or `None` if no time has passed.
    pub fn average(&self, time: f64) -> Option<f64> {
        let t = if time.is_finite() {
            time.max(self.last_time)
        } else {
            self.last_time
        };
        let span = t - self.start;
        (span > 0.0).then(|| (self.area + self.value * (t - self.last_time)) / span)
    }

    /// Restarts the average at `time`, keeping the current value (to discard a warm-up period).
    pub fn reset(&mut self, time: f64) {
        let t = if time.is_finite() {
            time.max(self.last_time)
        } else {
            self.last_time
        };
        *self = Self::new(t, self.value);
    }
}

/// The two-sided 95% critical value of Student's t distribution for `df` degrees of freedom.
///
/// Exact table values (to three decimals) for 1–30, 40, 60 and 120, linear interpolation in
/// 1/df between them, and 1.960 (the normal limit) as df grows without bound.
pub fn t_critical_95(df: u64) -> Option<f64> {
    const SMALL: [f64; 30] = [
        12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
        2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
        2.052, 2.048, 2.045, 2.042,
    ];
    const LARGE: [(f64, f64); 4] = [(30.0, 2.042), (40.0, 2.021), (60.0, 2.000), (120.0, 1.980)];
    match df {
        0 => None,
        1..=30 => SMALL.get(usize::try_from(df - 1).ok()?).copied(),
        _ => {
            let x = 1.0 / df as f64;
            // Pairs bracketing df, ending with the normal limit at 1/df = 0.
            let points = LARGE
                .iter()
                .map(|(d, t)| (1.0 / d, *t))
                .chain(std::iter::once((0.0, 1.960)));
            let pts: Vec<(f64, f64)> = points.collect();
            pts.windows(2).find_map(|w| {
                let ((x0, t0), (x1, t1)) = (w[0], w[1]);
                (x <= x0 && x >= x1).then(|| t1 + (t0 - t1) * (x - x1) / (x0 - x1))
            })
        }
    }
}

/// The mean of independent replication results with a 95% confidence interval.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Summary {
    pub n: u64,
    pub mean: f64,
    /// Sample standard deviation across replications (0 for a single one).
    pub std_dev: f64,
    /// Half-width of the 95% confidence interval for the mean (infinite for a single one).
    pub half_width: f64,
}

impl Summary {
    /// Summarises `values`, ignoring non-finite ones. `None` if none are left.
    pub fn of(values: &[f64]) -> Option<Self> {
        let mut t = Tally::new();
        values.iter().for_each(|v| t.add(*v));
        let mean = t.mean()?;
        let n = t.count();
        let std_dev = t.std_dev().unwrap_or(0.0);
        let half_width = match t_critical_95(n - 1) {
            Some(tc) => tc * std_dev / (n as f64).sqrt(),
            None => f64::INFINITY,
        };
        Some(Self {
            n,
            mean,
            std_dev,
            half_width,
        })
    }

    /// Whether `value` lies inside the 95% confidence interval.
    pub fn contains(&self, value: f64) -> bool {
        (value - self.mean).abs() <= self.half_width
    }
}

impl std::fmt::Display for Summary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.4} ± {:.4} (95% CI, n = {})",
            self.mean, self.half_width, self.n
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn tally_matches_two_pass_formulas() {
        let xs = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mut t = Tally::new();
        xs.iter().for_each(|x| t.add(*x));
        assert_eq!(t.count(), 8);
        assert!(t.mean().is_some_and(|m| close(m, 5.0)));
        // Sum of squared deviations is 32, so the sample variance is 32 / 7.
        assert!(t.variance().is_some_and(|v| close(v, 32.0 / 7.0)));
        assert_eq!((t.min(), t.max()), (Some(2.0), Some(9.0)));
    }

    #[test]
    fn tally_edge_cases() {
        let mut t = Tally::new();
        assert_eq!((t.mean(), t.variance(), t.min()), (None, None, None));
        t.add(f64::NAN);
        t.add(3.0);
        assert_eq!((t.count(), t.mean(), t.variance()), (1, Some(3.0), None));
    }

    #[test]
    fn time_weighted_average() {
        // 0 for 2 units, 3 for 1 unit, 1 for 1 unit: area 4 over 4 units.
        let mut q = TimeWeighted::new(0.0, 0.0);
        q.set(2.0, 3.0);
        q.set(3.0, 1.0);
        assert!(q.average(4.0).is_some_and(|a| close(a, 1.0)));
        assert_eq!(TimeWeighted::new(5.0, 2.0).average(5.0), None);
    }

    #[test]
    fn time_weighted_reset_discards_warmup() {
        let mut q = TimeWeighted::new(0.0, 10.0);
        q.set(5.0, 2.0);
        q.reset(5.0);
        q.set(7.0, 4.0);
        // 2 for 2 units, 4 for 2 units.
        assert!(q.average(9.0).is_some_and(|a| close(a, 3.0)));
    }

    #[test]
    fn t_table() {
        assert_eq!(t_critical_95(0), None);
        assert_eq!(t_critical_95(1), Some(12.706));
        assert_eq!(t_critical_95(7), Some(2.365));
        assert_eq!(t_critical_95(30), Some(2.042));
        assert!(t_critical_95(60).is_some_and(|t| close(t, 2.000)));
        // df 48 is 1/48 between 1/40 and 1/60: published value 2.011.
        assert!(t_critical_95(48).is_some_and(|t| (t - 2.011).abs() < 0.002));
        assert!(t_critical_95(1_000_000).is_some_and(|t| (t - 1.960).abs() < 0.001));
    }

    #[test]
    fn summary_interval() {
        let s = Summary::of(&[4.0, 6.0]).unwrap_or(Summary {
            n: 0,
            mean: 0.0,
            std_dev: 0.0,
            half_width: 0.0,
        });
        assert_eq!(s.n, 2);
        assert!(close(s.mean, 5.0));
        // std dev sqrt(2), half width 12.706 * sqrt(2) / sqrt(2).
        assert!(close(s.half_width, 12.706));
        assert!(s.contains(15.0) && !s.contains(18.0));
        assert_eq!(Summary::of(&[]), None);
        assert!(Summary::of(&[1.0]).is_some_and(|s| s.half_width.is_infinite()));
    }
}
