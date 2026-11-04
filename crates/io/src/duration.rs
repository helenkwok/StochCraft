//! ISO 8601 durations (`xs:duration`), as BPSim's `DurationParameter` uses them: `PT24M`,
//! `PT60H`, `P1M`, `P2W`, `P1DT2H30M15.5S`.

/// Seconds in a duration. Years count as 365 days and months as 30 days: BPSim gives scenario
/// lengths such as `P1M` without a calendar to resolve them against. Negative or malformed
/// durations give `None`.
pub fn seconds(s: &str) -> Option<f64> {
    let rest = s.trim().strip_prefix('P')?;
    if rest.is_empty() {
        return None;
    }
    let (date, time) = match rest.split_once('T') {
        Some((d, t)) if !t.is_empty() => (d, Some(t)),
        Some(_) => return None,
        None => (rest, None),
    };
    const DAY: f64 = 86_400.0;
    let date_units = [
        ('Y', 365.0 * DAY),
        ('M', 30.0 * DAY),
        ('W', 7.0 * DAY),
        ('D', DAY),
    ];
    let time_units = [('H', 3_600.0), ('M', 60.0), ('S', 1.0)];
    Some(fields(date, &date_units)? + time.map_or(Some(0.0), |t| fields(t, &time_units))?)
}

/// Sums `number unit` pairs that must appear in the order of `units`, each at most once.
fn fields(s: &str, units: &[(char, f64)]) -> Option<f64> {
    let mut total = 0.0;
    let mut number = String::new();
    let mut next_unit = 0;
    for c in s.chars() {
        if c.is_ascii_digit() || c == '.' || c == ',' {
            number.push(if c == ',' { '.' } else { c });
            continue;
        }
        let pos = units.iter().skip(next_unit).position(|(u, _)| *u == c)? + next_unit;
        let value: f64 = number.parse().ok()?;
        if !value.is_finite() {
            return None;
        }
        total += value * units.get(pos)?.1;
        number.clear();
        next_unit = pos + 1;
    }
    number.is_empty().then_some(total)
}

/// A duration in seconds written as `PT…S` (exact, no calendar units).
pub fn format_seconds(secs: f64) -> String {
    format!("PT{}S", trim_float(secs.max(0.0)))
}

/// A float without a trailing `.0` and without exponent notation.
pub fn trim_float(v: f64) -> String {
    let s = format!("{v}");
    s.strip_suffix(".0").map_or(s.clone(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpsim_examples() {
        assert_eq!(seconds("PT24M"), Some(1_440.0));
        assert_eq!(seconds("PT60H"), Some(216_000.0));
        assert_eq!(seconds("PT0M"), Some(0.0));
        assert_eq!(seconds("P1M"), Some(30.0 * 86_400.0));
        assert_eq!(seconds("P2W"), Some(14.0 * 86_400.0));
        assert_eq!(seconds("P1DT2H30M15.5S"), Some(86_400.0 + 9_015.5));
        assert_eq!(seconds("PT0,5S"), Some(0.5));
        assert_eq!(seconds(" P1Y "), Some(365.0 * 86_400.0));
    }

    #[test]
    fn malformed_durations() {
        for bad in [
            "", "P", "PT", "24M", "-PT1H", "PT1H2", "PT1M1H", "P1H", "PT1D", "PxM", "P1M1M",
            "PT1..5S",
        ] {
            assert_eq!(seconds(bad), None, "{bad}");
        }
    }

    #[test]
    fn formatting_round_trips() {
        for v in [0.0, 1.5, 3_600.0, 1e-3] {
            assert_eq!(seconds(&format_seconds(v)), Some(v));
        }
        assert_eq!(trim_float(2.0), "2");
        assert_eq!(trim_float(0.25), "0.25");
    }
}
