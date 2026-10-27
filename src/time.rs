// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Dates and times: the `date`, `datetime` and `time` values, time zones,
//! the functions `year()`, `month()`... and durations such as `7 days`.
//!
//! A `datetime` is an instant, kept in UTC to the millisecond. A time zone
//! only changes how it is shown and how a text without an offset is read:
//! the server's (`time.zone`), else the user's, else the session's
//! (`timezone ...`). A `date` and a `time` belong to no zone: a birthday or
//! an opening hour stays what it is wherever it is read.

use chrono::{
    DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, Offset, SecondsFormat,
    TimeZone, Timelike, Utc,
};
use serde_json::Value as Json;
use std::cell::RefCell;

/// Where dates are shown and read.
#[derive(Clone, Debug, PartialEq)]
pub enum Zone {
    Utc,
    /// The machine's own zone.
    Local,
    Named(chrono_tz::Tz),
    Fixed(FixedOffset),
}

impl Zone {
    /// `UTC`, `local`, a name such as `Africa/Kinshasa`, or an offset such as `+01:00`.
    pub fn parse(name: &str) -> Result<Zone, String> {
        let s = name.trim();
        if s.eq_ignore_ascii_case("utc") || s == "Z" {
            return Ok(Zone::Utc);
        }
        if s.eq_ignore_ascii_case("local") {
            return Ok(Zone::Local);
        }
        if s.starts_with(['+', '-']) {
            return parse_offset(s).map(Zone::Fixed).ok_or_else(|| bad_zone(name));
        }
        s.parse::<chrono_tz::Tz>().map(Zone::Named).map_err(|_| bad_zone(name))
    }

    pub fn name(&self) -> String {
        match self {
            Zone::Utc => "UTC".into(),
            Zone::Local => "local".into(),
            Zone::Named(tz) => tz.name().into(),
            Zone::Fixed(f) => f.to_string(),
        }
    }

    /// The offset from UTC at `instant`, summer time included.
    pub fn offset_at(&self, instant: DateTime<Utc>) -> FixedOffset {
        let naive = instant.naive_utc();
        match self {
            Zone::Utc => FixedOffset::east_opt(0).expect("zero offset"),
            Zone::Local => Local.offset_from_utc_datetime(&naive).fix(),
            Zone::Named(tz) => tz.offset_from_utc_datetime(&naive).fix(),
            Zone::Fixed(f) => *f,
        }
    }

    /// The wall clock of this zone at `instant`.
    pub fn wall(&self, instant: DateTime<Utc>) -> NaiveDateTime {
        instant.naive_utc() + Duration::seconds(self.offset_at(instant).local_minus_utc() as i64)
    }

    /// The instant a wall clock of this zone shows. When the clocks go back,
    /// the first of the two; when they go forward, the hour skipped is refused.
    pub fn instant(&self, wall: NaiveDateTime) -> Option<DateTime<Utc>> {
        match self {
            Zone::Utc => Some(wall.and_utc()),
            Zone::Local => Local.from_local_datetime(&wall).earliest().map(|d| d.with_timezone(&Utc)),
            Zone::Named(tz) => tz.from_local_datetime(&wall).earliest().map(|d| d.with_timezone(&Utc)),
            Zone::Fixed(f) => f.from_local_datetime(&wall).single().map(|d| d.with_timezone(&Utc)),
        }
    }
}

fn bad_zone(name: &str) -> String {
    format!("'{name}' is not a time zone: use UTC, local, a name such as Africa/Kinshasa, or an offset such as +01:00")
}

/// `+01:00`, `-0530`, `+01`.
fn parse_offset(s: &str) -> Option<FixedOffset> {
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let digits: String = s[1..].chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (digits[..2].parse::<i32>().ok()?, digits[2..].parse::<i32>().ok()?),
        _ => return None,
    };
    if h > 14 || m > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

thread_local! {
    /// The zone of the session running on this thread. Each session sets it
    /// before a command, so that instances in one process never mix theirs.
    static SESSION: RefCell<Option<Zone>> = const { RefCell::new(None) };
}

/// Sets the zone of the session about to run on this thread (`None`: the machine's).
pub fn set_session_zone(zone: Option<Zone>) {
    SESSION.with(|z| *z.borrow_mut() = zone);
}

/// The zone dates are shown and read in, on this thread.
pub fn zone() -> Zone {
    SESSION.with(|z| z.borrow().clone()).unwrap_or(Zone::Local)
}

pub fn now() -> DateTime<Utc> {
    let ms = crate::util::now_ms();
    DateTime::from_timestamp_millis(ms).unwrap_or_default()
}

/// Today, in the current zone: the day it already is in Kinshasa may still
/// be yesterday in Los Angeles.
pub fn today() -> NaiveDate {
    zone().wall(now()).date()
}

pub fn current_time() -> NaiveTime {
    let t = zone().wall(now()).time();
    // Kept to the millisecond, as datetimes are.
    t.with_nanosecond(t.nanosecond() / 1_000_000 * 1_000_000).unwrap_or(t)
}

/// How a `datetime` is stored and sent to programs: ISO 8601, UTC, to the
/// millisecond: `2026-10-27T13:30:00.000Z`.
pub fn iso(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// How the shell shows a `datetime`: `2026-10-27 14:30:00 +01:00` in the
/// current zone; milliseconds only when there are some.
pub fn show(instant: DateTime<Utc>) -> String {
    let zone = zone();
    let wall = zone.wall(instant);
    let offset = zone.offset_at(instant);
    format!("{} {offset}", wall_text(wall))
}

fn wall_text(wall: NaiveDateTime) -> String {
    if wall.nanosecond() >= 1_000_000 {
        wall.format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    } else {
        wall.format("%Y-%m-%d %H:%M:%S").to_string()
    }
}

pub fn date_text(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub fn time_text(t: NaiveTime) -> String {
    if t.nanosecond() >= 1_000_000 {
        t.format("%H:%M:%S%.3f").to_string()
    } else {
        t.format("%H:%M:%S").to_string()
    }
}

/// A moment given in seconds since 1970 (users, audit), in the current zone.
pub fn show_secs(secs: i64) -> String {
    match DateTime::from_timestamp(secs, 0) {
        Some(instant) => zone().wall(instant).format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "-".to_string(),
    }
}

const DATETIME_HELP: &str = "write a datetime as \"2026-10-27 14:30:00\" (seconds optional), with a T instead \
                             of the space if you like, and optionally a zone: Z, +01:00 or UTC";

/// Reads a datetime. Without a zone of its own, the text is a wall clock of
/// `zone`; a date alone is midnight there.
pub fn parse_datetime(text: &str, zone: &Zone) -> Result<DateTime<Utc>, String> {
    let s = text.trim();
    let invalid = || format!("'{text}' is not a datetime: {DATETIME_HELP}");
    // With an offset: 2026-10-27T13:30:00Z, 2026-10-27 14:30:00+01:00, 2026-10-27 14:30:00 +01:00.
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Ok(d.with_timezone(&Utc));
    }
    for f in ["%Y-%m-%d %H:%M:%S%.f %:z", "%Y-%m-%d %H:%M %:z", "%Y-%m-%dT%H:%M%:z", "%Y-%m-%d %H:%M%:z"] {
        if let Ok(d) = DateTime::parse_from_str(s, f) {
            return Ok(d.with_timezone(&Utc));
        }
    }
    let (s, zone) = match s.strip_suffix(" UTC").or_else(|| s.strip_suffix('Z')).or_else(|| s.strip_suffix('z')) {
        Some(rest) => (rest.trim_end(), &Zone::Utc),
        None => (s, zone),
    };
    let wall = wall_of(s).ok_or_else(invalid)?;
    zone.instant(wall).ok_or_else(|| {
        format!("'{text}' does not exist in the time zone {}: the clocks went forward then", zone.name())
    })
}

fn wall_of(s: &str) -> Option<NaiveDateTime> {
    let s = s.replacen('T', " ", 1);
    for f in ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M"] {
        if let Ok(w) = NaiveDateTime::parse_from_str(&s, f) {
            return Some(w);
        }
    }
    NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok().map(|d| d.and_time(NaiveTime::MIN))
}

pub fn parse_date(text: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d")
        .map_err(|_| format!("'{text}' is not a date: write it as \"2026-10-27\" (year, month, day)"))
}

pub fn parse_time(text: &str) -> Result<NaiveTime, String> {
    let s = text.trim();
    for f in ["%H:%M:%S%.f", "%H:%M"] {
        if let Ok(t) = NaiveTime::parse_from_str(s, f) {
            return Ok(t);
        }
    }
    Err(format!("'{text}' is not a time: write it as \"14:30\" or \"14:30:00\""))
}

/// A field of a document read as a moment: a datetime text (`2026-10-27T13:30:00Z`,
/// or a wall clock of the current zone), or a value `{"$datetime": ...}`.
pub fn instant_of(j: &Json) -> Option<DateTime<Utc>> {
    let text = match j {
        Json::String(s) => s.as_str(),
        Json::Object(m) if m.len() == 1 => m.get("$datetime")?.as_str()?,
        _ => return None,
    };
    parse_datetime(text, &zone()).ok()
}

/// A value of a condition that compares moments: `{"$datetime": "<iso>"}`.
pub fn datetime_json(instant: DateTime<Utc>) -> Json {
    serde_json::json!({ "$datetime": iso(instant) })
}

/// `{"$time": "08:00:00"}`, a value of a condition that compares times of day.
pub fn time_json(t: NaiveTime) -> Json {
    serde_json::json!({ "$time": time_text(t) })
}

/// A condition value as it is written: `datetime "2026-10-27 14:30:00 +01:00"`
/// for a moment, the JSON otherwise.
pub fn show_value(j: &Json) -> String {
    match tagged(j) {
        Some(("$datetime", text)) => match parse_datetime(text, &Zone::Utc) {
            Ok(at) => format!("datetime \"{}\"", show(at)),
            Err(_) => j.to_string(),
        },
        Some((_, text)) => format!("time \"{text}\""),
        None => j.to_string(),
    }
}

/// The kind of a condition value written with a date function, if it is one.
pub fn tagged(j: &Json) -> Option<(&'static str, &str)> {
    let m = j.as_object().filter(|m| m.len() == 1)?;
    ["$datetime", "$time"].into_iter().find_map(|tag| m.get(tag).and_then(Json::as_str).map(|s| (tag, s)))
}

/// Compares a field with a `$datetime` or `$time` value; `None` when the
/// field does not read as one.
pub fn compare_tagged(field: &Json, tag: &str, text: &str) -> Option<std::cmp::Ordering> {
    match tag {
        "$datetime" => Some(instant_of(field)?.cmp(&parse_datetime(text, &Zone::Utc).ok()?)),
        "$time" => {
            let f = match field {
                Json::String(s) => parse_time(s).ok()?,
                Json::Object(_) => parse_time(tagged(field).filter(|(t, _)| *t == "$time")?.1).ok()?,
                _ => return None,
            };
            Some(f.cmp(&parse_time(text).ok()?))
        }
        _ => None,
    }
}

/// `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()`, `weekday()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeFn {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    /// 1 for Monday to 7 for Sunday (ISO 8601).
    Weekday,
}

impl TimeFn {
    pub fn parse(name: &str) -> Option<TimeFn> {
        Some(match name.to_ascii_lowercase().as_str() {
            "year" => TimeFn::Year,
            "month" => TimeFn::Month,
            "day" => TimeFn::Day,
            "hour" => TimeFn::Hour,
            "minute" => TimeFn::Minute,
            "second" => TimeFn::Second,
            "weekday" => TimeFn::Weekday,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            TimeFn::Year => "year",
            TimeFn::Month => "month",
            TimeFn::Day => "day",
            TimeFn::Hour => "hour",
            TimeFn::Minute => "minute",
            TimeFn::Second => "second",
            TimeFn::Weekday => "weekday",
        }
    }

    fn of_date(self, d: NaiveDate) -> Option<i64> {
        Some(match self {
            TimeFn::Year => d.year() as i64,
            TimeFn::Month => d.month() as i64,
            TimeFn::Day => d.day() as i64,
            TimeFn::Weekday => d.weekday().number_from_monday() as i64,
            _ => return None,
        })
    }

    fn of_time(self, t: NaiveTime) -> Option<i64> {
        Some(match self {
            TimeFn::Hour => t.hour() as i64,
            TimeFn::Minute => t.minute() as i64,
            TimeFn::Second => t.second() as i64,
            _ => return None,
        })
    }

    /// The part of a date, time or datetime. A date has no hour, a time no
    /// year; a datetime is read in the current zone.
    pub fn of(self, v: &Json) -> Option<i64> {
        let text = match v {
            Json::String(s) => s.as_str(),
            Json::Object(_) => tagged(v)?.1,
            _ => return None,
        };
        if let Ok(d) = NaiveDate::parse_from_str(text.trim(), "%Y-%m-%d") {
            return self.of_date(d);
        }
        if let Ok(t) = parse_time(text) {
            return self.of_time(t);
        }
        let wall = zone().wall(parse_datetime(text, &zone()).ok()?);
        self.of_date(wall.date()).or_else(|| self.of_time(wall.time()))
    }

    pub fn of_value(self, v: &crate::value::Value) -> Option<i64> {
        use crate::value::Value;
        match v {
            Value::DateTime(i) => {
                let wall = zone().wall(*i);
                self.of_date(wall.date()).or_else(|| self.of_time(wall.time()))
            }
            Value::Date(d) => self.of_date(*d),
            Value::Time(t) => self.of_time(*t),
            Value::Str(s) => self.of(&Json::String(s.clone())),
            _ => None,
        }
    }
}

/// `7 days`, `2 hours`, `30 minutes`, `10 seconds`, `500 ms`, `1 week`.
pub fn duration(n: i64, unit: &str) -> Option<Duration> {
    let per = match unit.to_ascii_lowercase().as_str() {
        "ms" | "millisecond" | "milliseconds" => Duration::milliseconds(1),
        "s" | "sec" | "second" | "seconds" => Duration::seconds(1),
        "min" | "minute" | "minutes" => Duration::minutes(1),
        "h" | "hour" | "hours" => Duration::hours(1),
        "d" | "day" | "days" => Duration::days(1),
        "w" | "week" | "weeks" => Duration::weeks(1),
        _ => return None,
    };
    per.checked_mul(i32::try_from(n).ok()?)
}

/// The instants `[from, to)` of a year of the current zone, for `year(x) = 2026`.
pub fn year_bounds(year: i64) -> Option<(DateTime<Utc>, DateTime<Utc>)> {
    let start = |y: i64| {
        let d = NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, 1, 1)?;
        zone().instant(d.and_time(NaiveTime::MIN))
    };
    Some((start(year)?, start(year + 1)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zones_and_texts() {
        let kin = Zone::parse("Africa/Kinshasa").unwrap();
        let t = parse_datetime("2026-10-27 14:30:00", &kin).unwrap();
        assert_eq!(iso(t), "2026-10-27T13:30:00.000Z");
        for text in
            ["2026-10-27T13:30:00Z", "2026-10-27 14:30:00 +01:00", "2026-10-27T14:30+01:00", "2026-10-27 13:30 UTC"]
        {
            assert_eq!(parse_datetime(text, &Zone::Utc).unwrap(), t, "{text}");
        }
        assert_eq!(iso(parse_datetime("2026-10-27", &kin).unwrap()), "2026-10-26T23:00:00.000Z");
        assert!(parse_datetime("27/10/2026", &kin).unwrap_err().contains("is not a datetime"));
        // Summer time: New York is 4 hours behind UTC in October, 5 in December.
        let ny = Zone::parse("America/New_York").unwrap();
        assert_eq!(ny.offset_at(t).to_string(), "-04:00");
        assert_eq!(ny.offset_at(parse_datetime("2026-12-01 12:00Z", &Zone::Utc).unwrap()).to_string(), "-05:00");
        // The hour skipped when the clocks go forward does not exist.
        assert!(parse_datetime("2026-03-08 02:30", &ny).unwrap_err().contains("does not exist"));
        assert_eq!(Zone::parse("+05:30").unwrap().name(), "+05:30");
        assert!(Zone::parse("Mars/Olympus").is_err() && Zone::parse("+25:00").is_err());
    }

    #[test]
    fn functions_read_dates_times_and_datetimes() {
        set_session_zone(Some(Zone::parse("Africa/Kinshasa").unwrap()));
        let at = Json::String("2026-12-31T23:30:00Z".into());
        // Already the next year in Kinshasa.
        assert_eq!(TimeFn::Year.of(&at), Some(2027));
        assert_eq!(TimeFn::Hour.of(&at), Some(0));
        assert_eq!(TimeFn::Weekday.of(&Json::String("2026-10-27".into())), Some(2));
        assert_eq!(TimeFn::Hour.of(&Json::String("2026-10-27".into())), None);
        assert_eq!(TimeFn::Minute.of(&Json::String("08:45".into())), Some(45));
        assert_eq!(duration(7, "days"), Some(Duration::days(7)));
        assert_eq!(duration(1, "fortnight"), None);
        let (from, to) = year_bounds(2026).unwrap();
        assert_eq!((iso(from), iso(to)), ("2025-12-31T23:00:00.000Z".into(), "2026-12-31T23:00:00.000Z".into()));
        set_session_zone(None);
    }
}
