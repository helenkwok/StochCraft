//! Calendars: the iCalendar (RFC 5545) subset BPSim models use for shifts and opening hours.
//!
//! A calendar is a list of events, each a period (`DTSTART` to `DTEND`) that can repeat by an
//! `RRULE` with `FREQ=DAILY` or `FREQ=WEEKLY`, an `INTERVAL`, `BYDAY` (plain weekdays), `WKST`,
//! and an end by `UNTIL` or `COUNT`. A calendar is active while any occurrence of any of its
//! events is under way. Times are civil (floating) date-times: a `Z` suffix and `TZID` are read
//! as the same wall-clock time, so a whole model is in one time zone.
//!
//! Model time 0 is the scenario's start ([`crate::Experiment::start`]); without one, midnight
//! of the day the earliest event starts.

use crate::{ElementId, TimeUnit};

/// A civil date and time, to the second: seconds since 1970-01-01T00:00:00 on the same clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime(pub i64);

const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

/// Days since 1970-01-01 of a proleptic Gregorian date: Howard Hinnant's `days_from_civil`
/// (public domain; ATTRIBUTION.md), counting from March so the leap day comes last.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = i64::from(m);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day count from [`days_from_civil`]: Hinnant's `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
        _ => 28,
    }
}

impl DateTime {
    /// A date and time, if it exists.
    pub fn new(y: i64, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> Option<Self> {
        if !(1..=12).contains(&mo) || d == 0 || d > days_in_month(y, mo) || h > 23 || mi > 59 {
            return None;
        }
        // A leap second is read as the last second of its minute.
        let s = s.min(59);
        Some(Self(
            days_from_civil(y, mo, d) * DAY + i64::from(h * 3600 + mi * 60 + s),
        ))
    }

    /// Reads iCalendar's `20160101T060000` (with or without a trailing `Z`) or a date alone,
    /// `20160101` (its midnight).
    pub fn parse_ical(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_suffix('Z').unwrap_or(s);
        let digits = |t: &str| t.bytes().all(|b| b.is_ascii_digit());
        let (date, time) = match s.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        if date.len() != 8 || !digits(date) {
            return None;
        }
        let n = |a: &str| a.parse::<u32>().ok();
        let (y, mo, d) = (date[..4].parse().ok()?, n(&date[4..6])?, n(&date[6..8])?);
        let (h, mi, sec) = match time {
            Some(t) if t.len() == 6 && digits(t) => (n(&t[..2])?, n(&t[2..4])?, n(&t[4..6])?),
            Some(_) => return None,
            None => (0, 0, 0),
        };
        Self::new(y, mo, d, h, mi, sec)
    }

    /// Reads XML Schema's `2016-01-01T06:00:00` (BPSim's `DateTimeParameter`), with optional
    /// fractional seconds and time zone, which are dropped; or a date alone.
    pub fn parse_iso(s: &str) -> Option<Self> {
        let s = s.trim();
        let (date, time) = match s.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        let mut d = date.splitn(3, '-');
        let y: i64 = d.next()?.parse().ok()?;
        let mo: u32 = d.next()?.parse().ok()?;
        let day: u32 = d.next()?.get(..2)?.parse().ok()?;
        let (h, mi, sec) = match time {
            Some(t) => {
                let t = t.trim_end_matches('Z');
                let t = t.split(['+', '-']).next().unwrap_or(t);
                let mut p = t.split(':');
                let h: u32 = p.next()?.parse().ok()?;
                let mi: u32 = p.next()?.parse().ok()?;
                let sec: f64 = p.next().unwrap_or("0").parse().ok()?;
                (h, mi, sec as u32)
            }
            None => (0, 0, 0),
        };
        Self::new(y, mo, day, h, mi, sec)
    }

    fn parts(self) -> (i64, u32, u32, i64) {
        let (y, m, d) = civil_from_days(self.0.div_euclid(DAY));
        (y, m, d, self.0.rem_euclid(DAY))
    }

    /// As iCalendar writes it: `20160101T060000`.
    pub fn ical(self) -> String {
        let (y, m, d, s) = self.parts();
        format!(
            "{y:04}{m:02}{d:02}T{:02}{:02}{:02}",
            s / 3600,
            s / 60 % 60,
            s % 60
        )
    }

    /// As XML Schema writes it: `2016-01-01T06:00:00`.
    pub fn iso(self) -> String {
        let (y, m, d, s) = self.parts();
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
            s / 3600,
            s / 60 % 60,
            s % 60
        )
    }

    /// The weekday, 0 for Monday to 6 for Sunday.
    pub fn weekday(self) -> u8 {
        // 1970-01-01 was a Thursday.
        ((self.0.div_euclid(DAY) + 3).rem_euclid(7)) as u8
    }

    /// Midnight of the same day.
    pub fn midnight(self) -> Self {
        Self(self.0.div_euclid(DAY) * DAY)
    }
}

/// The weekday codes of iCalendar, Monday first.
pub const WEEKDAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// How often an event repeats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
}

/// An `RRULE`, in the subset StochCraft simulates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    /// Every this many days or weeks (at least 1).
    pub interval: u32,
    /// The weekdays it falls on (0 for Monday), in order; empty for every day (daily) or the
    /// weekday of the start (weekly).
    pub by_day: Vec<u8>,
    /// The day a week starts on, for weekly rules with an interval over 1 (Monday by default).
    pub week_start: u8,
    /// The last time an occurrence can start (inclusive).
    pub until: Option<DateTime>,
    /// How many occurrences there are, the first included.
    pub count: Option<u32>,
}

impl Rule {
    /// Reads an `RRULE` value such as `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;UNTIL=20161231T235959`.
    /// Anything outside the subset is an error naming it.
    pub fn parse(s: &str) -> Result<Self, String> {
        let mut rule = Rule {
            freq: Freq::Daily,
            interval: 1,
            by_day: Vec::new(),
            week_start: 0,
            until: None,
            count: None,
        };
        let mut freq = None;
        for part in s.trim().split(';').filter(|p| !p.trim().is_empty()) {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| format!("`{part}` is not NAME=value"))?;
            let (k, v) = (k.trim().to_ascii_uppercase(), v.trim());
            match k.as_str() {
                "FREQ" => {
                    freq = Some(match v.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        other => {
                            return Err(format!(
                                "FREQ={other} is not supported (only DAILY and WEEKLY)"
                            ));
                        }
                    });
                }
                "INTERVAL" => {
                    rule.interval = v.parse().ok().filter(|&n: &u32| n >= 1).ok_or_else(|| {
                        format!("INTERVAL={v} is not a whole number of at least 1")
                    })?;
                }
                "BYDAY" => {
                    for d in v.split(',') {
                        let d = d.trim().to_ascii_uppercase();
                        let i = WEEKDAYS.iter().position(|w| *w == d).ok_or_else(|| {
                            format!("BYDAY={d} is not supported (only plain weekdays such as MO)")
                        })?;
                        if !rule.by_day.contains(&(i as u8)) {
                            rule.by_day.push(i as u8);
                        }
                    }
                    rule.by_day.sort_unstable();
                }
                "WKST" => {
                    let d = v.to_ascii_uppercase();
                    rule.week_start = WEEKDAYS
                        .iter()
                        .position(|w| *w == d)
                        .ok_or_else(|| format!("WKST={v} is not a weekday"))?
                        as u8;
                }
                "UNTIL" => {
                    rule.until = Some(
                        DateTime::parse_ical(v)
                            .ok_or_else(|| format!("UNTIL={v} is not a date or date-time"))?,
                    );
                }
                "COUNT" => {
                    rule.count = Some(
                        v.parse()
                            .map_err(|_| format!("COUNT={v} is not a whole number"))?,
                    );
                }
                other => return Err(format!("{other} is not supported in a repeat rule")),
            }
        }
        rule.freq = freq.ok_or("a repeat rule needs FREQ")?;
        if rule.until.is_some() && rule.count.is_some() {
            return Err("a repeat rule cannot have both UNTIL and COUNT".into());
        }
        Ok(rule)
    }

    /// The rule as iCalendar writes it.
    pub fn text(&self) -> String {
        let mut out = vec![format!(
            "FREQ={}",
            match self.freq {
                Freq::Daily => "DAILY",
                Freq::Weekly => "WEEKLY",
            }
        )];
        if self.interval != 1 {
            out.push(format!("INTERVAL={}", self.interval));
        }
        if !self.by_day.is_empty() {
            let days: Vec<&str> = self
                .by_day
                .iter()
                .filter_map(|&d| WEEKDAYS.get(usize::from(d)).copied())
                .collect();
            out.push(format!("BYDAY={}", days.join(",")));
        }
        if self.week_start != 0 {
            out.push(format!(
                "WKST={}",
                WEEKDAYS.get(usize::from(self.week_start)).unwrap_or(&"MO")
            ));
        }
        if let Some(u) = self.until {
            out.push(format!("UNTIL={}", u.ical()));
        }
        if let Some(c) = self.count {
            out.push(format!("COUNT={c}"));
        }
        out.join(";")
    }
}

/// A period that may repeat (an iCalendar `VEVENT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub start: DateTime,
    /// After `start`; each occurrence lasts `end - start`.
    pub end: DateTime,
    pub rule: Option<Rule>,
}

/// A safety limit on the occurrences one event can expand to in a run.
const MAX_OCCURRENCES: usize = 1_000_000;

impl Event {
    /// The starts of this event's occurrences, in order, that begin before `to` and end after
    /// `from`, as offsets in seconds.
    fn occurrences(&self, from: i64, to: i64, out: &mut Vec<(i64, i64)>) {
        let len = self.end.0 - self.start.0;
        if len <= 0 {
            return;
        }
        let Some(rule) = &self.rule else {
            if self.start.0 < to && self.end.0 > from {
                out.push((self.start.0, self.end.0));
            }
            return;
        };
        let time = self.start.0.rem_euclid(DAY);
        let start_day = self.start.0.div_euclid(DAY);
        let mut emitted = 0u32;
        let mut push = |s: i64, out: &mut Vec<(i64, i64)>| -> bool {
            // false: the rule has ended.
            if rule.count.is_some_and(|c| emitted >= c)
                || rule.until.is_some_and(|u| s > u.0)
                || s >= to
                || out.len() >= MAX_OCCURRENCES
            {
                return false;
            }
            emitted += 1;
            if s + len > from {
                out.push((s, s + len));
            }
            true
        };
        let step = i64::from(rule.interval);
        match rule.freq {
            Freq::Daily => {
                let mut day = start_day;
                loop {
                    if day * DAY >= to {
                        return;
                    }
                    let wd = (day + 3).rem_euclid(7) as u8;
                    if (rule.by_day.is_empty() || rule.by_day.contains(&wd))
                        && !push(day * DAY + time, out)
                    {
                        return;
                    }
                    day += step;
                }
            }
            Freq::Weekly => {
                let days: Vec<u8> = if rule.by_day.is_empty() {
                    vec![self.start.weekday()]
                } else {
                    rule.by_day.clone()
                };
                // The first day of the week DTSTART is in, by WKST.
                let wd = i64::from(self.start.weekday());
                let back = (wd - i64::from(rule.week_start)).rem_euclid(7);
                let mut week = start_day - back;
                loop {
                    // The week's days in time order from its start.
                    let mut in_week: Vec<i64> = days
                        .iter()
                        .map(|&d| week + (i64::from(d) - i64::from(rule.week_start)).rem_euclid(7))
                        .collect();
                    in_week.sort_unstable();
                    for day in in_week {
                        if day < start_day {
                            continue;
                        }
                        if !push(day * DAY + time, out) {
                            return;
                        }
                    }
                    week += step * 7;
                    if week * DAY > to + WEEK {
                        return;
                    }
                }
            }
        }
    }
}

/// A named calendar (BPSim `Calendar`): parameter values `validFor` it apply while it is
/// active.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Calendar {
    pub id: ElementId,
    pub name: String,
    pub events: Vec<Event>,
}

/// What reading a `VCALENDAR` found: its events, and notes on what it left out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Parsed {
    pub events: Vec<Event>,
    /// Properties that do not change when the calendar is active (UID, DTSTAMP, SUMMARY, …)
    /// are skipped silently; these are the ones that would, and are not simulated.
    pub ignored: Vec<String>,
    /// Events left out, with why.
    pub errors: Vec<String>,
}

/// Properties of a `VEVENT` that describe it without changing when it happens.
const DESCRIPTIVE: &[&str] = &[
    "UID",
    "DTSTAMP",
    "SUMMARY",
    "DESCRIPTION",
    "LOCATION",
    "CATEGORIES",
    "CLASS",
    "COMMENT",
    "CREATED",
    "LAST-MODIFIED",
    "SEQUENCE",
    "STATUS",
    "TRANSP",
    "PRIORITY",
    "ORGANIZER",
    "ATTENDEE",
    "CONTACT",
    "URL",
    "GEO",
    "RESOURCES",
];

/// Reads the text of a BPSim `Calendar`: a `VCALENDAR` with `VEVENT`s (a bare `VEVENT` list is
/// read too). Lines may be indented, as XML files indent them, and folded (RFC 5545 3.1).
pub fn parse(text: &str) -> Parsed {
    // Unfold: a line starting with a space or tab continues the previous one. Indentation
    // from the XML around it is removed first, from every line alike.
    // The first line often follows the XML tag directly, so it does not count for the indent;
    // and a line that reads as a property (`NAME:` or `NAME;`) is never a continuation.
    let lines: Vec<&str> = text.lines().collect();
    let indent = lines
        .iter()
        .skip(1)
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let looks_like_property = |l: &str| {
        let name: String = l
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        !name.is_empty()
            && name.chars().all(|c| !c.is_ascii_lowercase())
            && matches!(l[name.len()..].chars().next(), Some(':' | ';'))
    };
    let mut logical: Vec<String> = Vec::new();
    for l in lines {
        let l = l.trim_end_matches('\r');
        if l.trim().is_empty() {
            continue;
        }
        let l = l.get(indent..).unwrap_or(l.trim_start());
        if (l.starts_with(' ') || l.starts_with('\t'))
            && !looks_like_property(l.trim_start())
            && let Some(last) = logical.last_mut()
        {
            // Folded: XML indentation makes the fold's own space impossible to tell apart.
            last.push_str(l.trim_start());
        } else {
            logical.push(l.trim().to_owned());
        }
    }
    let mut out = Parsed::default();
    let mut current: Option<Vec<(String, String)>> = None;
    let mut depth_other = 0u32;
    for line in logical {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        // Parameters (`DTSTART;TZID=Europe/Paris`) follow the name after `;`.
        let (name, params) = match name.split_once(';') {
            Some((n, p)) => (n.trim().to_ascii_uppercase(), p.to_owned()),
            None => (name.trim().to_ascii_uppercase(), String::new()),
        };
        let value = value.trim().to_owned();
        match (name.as_str(), value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if depth_other == 0 => current = Some(Vec::new()),
            ("END", "VEVENT") if depth_other == 0 => {
                if let Some(props) = current.take() {
                    match event(&props, &mut out.ignored) {
                        Ok(e) => out.events.push(e),
                        Err(why) => out.errors.push(why),
                    }
                }
            }
            ("BEGIN", "VCALENDAR") | ("END", "VCALENDAR") => {}
            ("BEGIN", other) => {
                depth_other += 1;
                let what = format!("{other} components");
                if !out.ignored.contains(&what) {
                    out.ignored.push(what);
                }
            }
            ("END", _) => depth_other = depth_other.saturating_sub(1),
            _ => {
                if let Some(props) = current.as_mut().filter(|_| depth_other == 0) {
                    let n = if params.is_empty() {
                        name
                    } else {
                        format!("{name};{params}")
                    };
                    props.push((n, value));
                }
            }
        }
    }
    out
}

fn event(props: &[(String, String)], ignored: &mut Vec<String>) -> Result<Event, String> {
    let mut start = None;
    let mut end = None;
    let mut rule = None;
    let note = |what: String, ignored: &mut Vec<String>| {
        if !ignored.contains(&what) {
            ignored.push(what);
        }
    };
    for (name, value) in props {
        let (base, params) = name.split_once(';').unwrap_or((name, ""));
        if params.to_ascii_uppercase().contains("TZID") {
            note(
                "time zones (TZID): times are read as wall-clock times".into(),
                ignored,
            );
        }
        match base {
            "DTSTART" => {
                start = Some(
                    DateTime::parse_ical(value)
                        .ok_or_else(|| format!("DTSTART `{value}` is not a date-time"))?,
                );
            }
            "DTEND" => {
                end = Some(
                    DateTime::parse_ical(value)
                        .ok_or_else(|| format!("DTEND `{value}` is not a date-time"))?,
                );
            }
            "RRULE" => {
                if rule.is_some() {
                    return Err("an event with several RRULEs is not supported".into());
                }
                rule = Some(Rule::parse(value).map_err(|e| format!("RRULE `{value}`: {e}"))?);
            }
            b if DESCRIPTIVE.contains(&b) || b.starts_with("X-") => {}
            other => note(other.to_owned(), ignored),
        }
    }
    let start = start.ok_or("an event has no DTSTART")?;
    // RFC 5545: with no DTEND, an event on a date lasts the day.
    let end = end.unwrap_or(DateTime(start.0 + DAY));
    if end <= start {
        return Err(format!(
            "an event ends ({}) before it starts ({})",
            end.ical(),
            start.ical()
        ));
    }
    Ok(Event { start, end, rule })
}

/// The calendar as BPSim files carry it: a `VCALENDAR` with one `VEVENT` per event.
pub fn write(events: &[Event]) -> String {
    let mut s = String::from("BEGIN:VCALENDAR\nVERSION:2.0\nPRODID:-//StochCraft//EN\n");
    for e in events {
        s.push_str("BEGIN:VEVENT\n");
        s.push_str(&format!(
            "DTSTART:{}\nDTEND:{}\n",
            e.start.ical(),
            e.end.ical()
        ));
        if let Some(r) = &e.rule {
            s.push_str(&format!("RRULE:{}\n", r.text()));
        }
        s.push_str("END:VEVENT\n");
    }
    s.push_str("END:VCALENDAR");
    s
}

impl Calendar {
    /// Where model time 0 falls for calendars without a scenario start: midnight of the day the
    /// earliest event starts.
    pub fn default_origin(calendars: &[Calendar]) -> Option<DateTime> {
        calendars
            .iter()
            .flat_map(|c| &c.events)
            .map(|e| e.start)
            .min()
            .map(DateTime::midnight)
    }

    /// The periods the calendar is active in model time from 0 to `horizon`, in `unit`, with
    /// time 0 at `origin`: sorted, with overlapping and touching periods merged.
    pub fn periods(&self, origin: DateTime, unit: TimeUnit, horizon: f64) -> Vec<(f64, f64)> {
        let per = unit.seconds();
        let to = origin.0 + (horizon * per).ceil().min(1e15) as i64 + 1;
        let mut raw = Vec::new();
        for e in &self.events {
            e.occurrences(origin.0, to, &mut raw);
        }
        raw.sort_unstable();
        let mut merged: Vec<(i64, i64)> = Vec::new();
        for (s, e) in raw {
            match merged.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => merged.push((s, e)),
            }
        }
        merged
            .into_iter()
            .map(|(s, e)| {
                (
                    ((s - origin.0) as f64 / per).max(0.0),
                    (e - origin.0) as f64 / per,
                )
            })
            .filter(|(s, e)| e > s)
            .collect()
    }
}

/// A parameter value that applies while a calendar is active (BPSim `validFor`).
#[derive(Debug, Clone, PartialEq)]
pub struct Timed<T> {
    pub calendar: ElementId,
    pub value: T,
}

/// Which value applies at each time: `default` outside every calendar, else the first of
/// `timed` whose calendar is active. Returns the changes from time 0 to `horizon`: (time, index
/// into `timed` or `None` for the default), the first at 0.
pub fn timeline<T>(
    timed: &[Timed<T>],
    periods: &dyn Fn(&ElementId) -> Vec<(f64, f64)>,
    horizon: f64,
) -> Vec<(f64, Option<usize>)> {
    let each: Vec<Vec<(f64, f64)>> = timed.iter().map(|t| periods(&t.calendar)).collect();
    let mut cuts: Vec<f64> = vec![0.0];
    for p in &each {
        for &(s, e) in p {
            cuts.push(s);
            cuts.push(e);
        }
    }
    cuts.retain(|t| *t >= 0.0 && *t <= horizon);
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let active_at = |t: f64| {
        each.iter()
            .position(|p| p.iter().any(|&(s, e)| s <= t && t < e))
    };
    let mut out: Vec<(f64, Option<usize>)> = Vec::new();
    for t in cuts {
        let v = active_at(t);
        if out.last().is_none_or(|(_, last)| *last != v) {
            out.push((t, v));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime {
        DateTime::parse_ical(s).unwrap_or(DateTime(i64::MIN))
    }

    #[test]
    fn dates_count_days_as_the_gregorian_calendar_does() {
        assert_eq!(dt("19700101T000000").0, 0);
        assert_eq!(dt("20000301").0, days_from_civil(2000, 3, 1) * DAY);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
        assert!(DateTime::new(2023, 2, 29, 0, 0, 0).is_none());
        // 2016-01-01 was a Friday.
        assert_eq!(dt("20160101T060000").weekday(), 4);
        assert_eq!(dt("20160101T060000").ical(), "20160101T060000");
        assert_eq!(dt("20160101T060000").iso(), "2016-01-01T06:00:00");
        assert_eq!(
            DateTime::parse_iso("2016-01-01T06:00:00"),
            Some(dt("20160101T060000"))
        );
        assert_eq!(
            DateTime::parse_iso("2016-01-01T06:00:00.5+02:00"),
            Some(dt("20160101T060000"))
        );
    }

    #[test]
    fn rules_read_and_write_back() {
        let r = Rule::parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=FR,MO;UNTIL=20161231T235959");
        assert_eq!(
            r.as_ref().map(Rule::text).as_deref(),
            Ok("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,FR;UNTIL=20161231T235959")
        );
        assert!(Rule::parse("FREQ=MONTHLY").is_err_and(|e| e.contains("MONTHLY")));
        assert!(Rule::parse("FREQ=DAILY;BYHOUR=9").is_err_and(|e| e.contains("BYHOUR")));
        assert!(Rule::parse("FREQ=WEEKLY;BYDAY=1MO").is_err());
        assert!(Rule::parse("FREQ=DAILY;COUNT=2;UNTIL=20160101").is_err());
        assert!(Rule::parse("INTERVAL=2").is_err());
    }

    /// The technical support model's shift from 10 pm to 1 am, every day, as the file indents it.
    #[test]
    fn an_indented_calendar_reads_and_repeats_overnight() {
        let text = "
            BEGIN:VCALENDAR
            BEGIN:VEVENT
            DTSTAMP:20160101T000000
            UID:1356035246845@localhost
            DTSTART:20160101T220000
            DTEND:20160102T010000
            RRULE:FREQ=DAILY
            END:VEVENT
            PRODID:PAF Editor
            VERSION:2.0
            END:VCALENDAR
        ";
        let p = parse(text);
        assert!(p.errors.is_empty() && p.ignored.is_empty(), "{p:?}");
        let c = Calendar {
            id: ElementId::unchecked("C6"),
            name: String::new(),
            events: p.events,
        };
        // From 6 am on the first day, in minutes, over two days.
        let periods = c.periods(dt("20160101T060000"), TimeUnit::Minutes, 2.0 * 1440.0);
        assert_eq!(periods, [(960.0, 1140.0), (2400.0, 2580.0)]);
        // Written and read back, it is the same; so it is as an XML file may indent it, with a
        // folded line.
        assert_eq!(parse(&write(&c.events)).events, c.events);
        let indented = "BEGIN:VCALENDAR\n    BEGIN:VEVENT\n    DTSTART:20160101T220000\n    DTEND:2016010\n     2T010000\n      RRULE:FREQ=DAILY\n    END:VEVENT\n  END:VCALENDAR";
        assert_eq!(parse(indented).events, c.events);
    }

    #[test]
    fn weekly_rules_fall_on_their_days_and_stop_when_told() {
        // Weekdays 9 to 5 from Monday 2016-01-04, for 7 occurrences.
        let e = Event {
            start: dt("20160104T090000"),
            end: dt("20160104T170000"),
            rule: Rule::parse("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;COUNT=7").ok(),
        };
        let mut out = Vec::new();
        e.occurrences(0, i64::MAX / 2, &mut out);
        let days: Vec<u8> = out.iter().map(|&(s, _)| DateTime(s).weekday()).collect();
        assert_eq!(days, [0, 1, 2, 3, 4, 0, 1]);
        // Every other Saturday from 9 January until 1 February: the 9th and the 23rd.
        let e = Event {
            start: dt("20160109T080000"),
            end: dt("20160109T120000"),
            rule: Rule::parse("FREQ=WEEKLY;INTERVAL=2;UNTIL=20160201").ok(),
        };
        let mut out = Vec::new();
        e.occurrences(0, i64::MAX / 2, &mut out);
        let starts: Vec<String> = out.iter().map(|&(s, _)| DateTime(s).ical()).collect();
        assert_eq!(starts, ["20160109T080000", "20160123T080000"]);
        // Daily on weekdays only.
        let e = Event {
            start: dt("20160101T090000"),
            end: dt("20160101T100000"),
            rule: Rule::parse("FREQ=DAILY;BYDAY=MO,FR;UNTIL=20160112").ok(),
        };
        let mut out = Vec::new();
        e.occurrences(0, i64::MAX / 2, &mut out);
        let starts: Vec<String> = out.iter().map(|&(s, _)| DateTime(s).ical()).collect();
        assert_eq!(
            starts,
            [
                "20160101T090000",
                "20160104T090000",
                "20160108T090000",
                "20160111T090000"
            ]
        );
    }

    #[test]
    fn what_cannot_be_simulated_is_reported() {
        let p = parse(
            "BEGIN:VCALENDAR\nBEGIN:VTIMEZONE\nTZID:X\nEND:VTIMEZONE\nBEGIN:VEVENT\nDTSTART;TZID=X:20160101T090000\nDTEND:20160101T100000\nEXDATE:20160102T090000\nEND:VEVENT\nBEGIN:VEVENT\nDTSTART:20160101T090000\nRRULE:FREQ=YEARLY\nEND:VEVENT\nEND:VCALENDAR",
        );
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.errors.len(), 1, "{p:?}");
        assert!(p.ignored.iter().any(|i| i == "EXDATE"), "{p:?}");
        assert!(p.ignored.iter().any(|i| i.contains("VTIMEZONE")), "{p:?}");
        assert!(p.ignored.iter().any(|i| i.contains("TZID")), "{p:?}");
    }

    #[test]
    fn the_first_active_calendar_chooses_the_value() {
        let periods = |c: &ElementId| match c.as_str() {
            "a" => vec![(1.0, 3.0)],
            "b" => vec![(2.0, 5.0)],
            _ => vec![],
        };
        let timed = [
            Timed {
                calendar: ElementId::unchecked("a"),
                value: 1,
            },
            Timed {
                calendar: ElementId::unchecked("b"),
                value: 2,
            },
        ];
        assert_eq!(
            timeline(&timed, &periods, 10.0),
            [(0.0, None), (1.0, Some(0)), (3.0, Some(1)), (5.0, None)]
        );
    }
}
