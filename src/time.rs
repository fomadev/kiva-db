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

/// What a format writes: a `date`, a `time` or a `datetime`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FormatKind {
    Date,
    Time,
    DateTime,
}

impl FormatKind {
    pub const ALL: [FormatKind; 3] = [FormatKind::Date, FormatKind::Time, FormatKind::DateTime];

    pub fn name(self) -> &'static str {
        match self {
            FormatKind::Date => "date",
            FormatKind::Time => "time",
            FormatKind::DateTime => "datetime",
        }
    }

    pub fn parse(s: &str) -> Option<FormatKind> {
        FormatKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s))
    }

    /// The setting of the server: `time.date_format`...
    pub fn config_key(self) -> &'static str {
        match self {
            FormatKind::Date => "date_format",
            FormatKind::Time => "time_format",
            FormatKind::DateTime => "datetime_format",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Millis,
    Offset,
    Sep(char),
}

/// Longest first, so that `SSS` is not read as something shorter.
const TOKENS: [(&str, Part); 8] = [
    ("YYYY", Part::Year),
    ("SSS", Part::Millis),
    ("XXX", Part::Offset),
    ("MM", Part::Month),
    ("DD", Part::Day),
    ("HH", Part::Hour),
    ("mm", Part::Minute),
    ("ss", Part::Second),
];
const SEPARATORS: &str = "-/.: T";
const TOKEN_HELP: &str = "tokens YYYY, MM, DD, HH, mm, ss, SSS, XXX, separated by - / . : space or T";

/// A format of dates such as `DD-MM-YYYY`: how a session writes and reads
/// them. It never changes what is stored, nor what programs receive.
#[derive(Clone, Debug, PartialEq)]
pub struct Pattern {
    text: String,
    parts: Vec<Part>,
}

/// The fields a text gave.
#[derive(Default)]
struct Fields {
    year: Option<i32>,
    month: Option<u32>,
    day: Option<u32>,
    hour: Option<u32>,
    minute: Option<u32>,
    second: Option<u32>,
    millis: Option<u32>,
    offset: Option<FixedOffset>,
}

impl Pattern {
    /// Reads and checks a format: each field once, those of its kind only,
    /// `HH mm ss SSS` in that order, `XXX` last.
    pub fn parse(kind: FormatKind, text: &str) -> Result<Pattern, String> {
        let bad = |why: String| format!("'{text}' is not a {} format: {why}", kind.name());
        let mut parts = Vec::new();
        let mut rest = text;
        'next: while let Some(c) = rest.chars().next() {
            for (token, part) in TOKENS {
                if let Some(r) = rest.strip_prefix(token) {
                    parts.push(part);
                    rest = r;
                    continue 'next;
                }
            }
            if rest.starts_with("YY") {
                return Err(bad("the year has four digits, YYYY".into()));
            }
            if !SEPARATORS.contains(c) {
                return Err(bad(format!("'{c}' is not one of the {TOKEN_HELP}")));
            }
            parts.push(Part::Sep(c));
            rest = &rest[c.len_utf8()..];
        }
        let fields: Vec<Part> = parts.iter().copied().filter(|p| !matches!(p, Part::Sep(_))).collect();
        let token = |p: Part| TOKENS.iter().find(|t| t.1 == p).map(|t| t.0).unwrap_or("");
        let pos = |p: Part| fields.iter().position(|q| *q == p);
        for p in &fields {
            if fields.iter().filter(|q| *q == p).count() > 1 {
                return Err(bad(format!("{} is there twice", token(*p))));
            }
        }
        use Part::*;
        let (needed, allowed): (&[Part], &[Part]) = match kind {
            FormatKind::Date => (&[Year, Month, Day], &[Year, Month, Day]),
            FormatKind::Time => (&[Hour, Minute], &[Hour, Minute, Second, Millis]),
            FormatKind::DateTime => {
                (&[Year, Month, Day, Hour, Minute], &[Year, Month, Day, Hour, Minute, Second, Millis, Offset])
            }
        };
        if let Some(p) = needed.iter().find(|p| pos(**p).is_none()) {
            return Err(bad(format!("{} is missing", token(*p))));
        }
        if let Some(p) = fields.iter().find(|p| !allowed.contains(p)) {
            return Err(bad(format!("a {} has no {}", kind.name(), token(*p))));
        }
        let ordered = |a: Part, b: Part| match (pos(a), pos(b)) {
            (Some(x), Some(y)) => x < y,
            _ => true,
        };
        if !ordered(Hour, Minute) || !ordered(Minute, Second) || !ordered(Second, Millis) {
            return Err(bad("the time is written HH, mm, then ss, then SSS".into()));
        }
        if pos(Millis).is_some() && pos(Second).is_none() {
            return Err(bad("SSS needs ss before it".into()));
        }
        if pos(Offset).is_some_and(|p| p + 1 != fields.len()) {
            return Err(bad("XXX comes last".into()));
        }
        // A text starting with the year is read as ISO (YYYY-MM-DD) whatever
        // the format: with the day before the month, 2026-03-04 would be two days.
        if pos(Year) == Some(0) && !ordered(Month, Day) {
            return Err(bad("with the year first, the month comes before the day, as in ISO".into()));
        }
        Ok(Pattern { text: text.to_string(), parts })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The fields of a text that follows the format exactly, digit for digit.
    /// The offset (`XXX`) may be left out: the text is then a wall clock.
    fn read(&self, text: &str) -> Option<Fields> {
        let b = text.trim().as_bytes();
        let mut i = 0;
        let mut f = Fields::default();
        let number = |i: &mut usize, width: usize| -> Option<u32> {
            let digits = b.get(*i..*i + width)?;
            *i += width;
            digits.iter().try_fold(0u32, |n, c| c.is_ascii_digit().then(|| n * 10 + u32::from(c - b'0')))
        };
        for (n, part) in self.parts.iter().enumerate() {
            let only_offset_left = self.parts[n..].iter().all(|p| matches!(p, Part::Sep(_) | Part::Offset));
            if i == b.len() && only_offset_left && self.parts[n..].contains(&Part::Offset) {
                break;
            }
            match part {
                Part::Sep(c) => {
                    if b.get(i) != Some(&(*c as u8)) {
                        return None;
                    }
                    i += 1;
                }
                Part::Offset => {
                    if b.get(i).is_some_and(|c| *c == b'Z' || *c == b'z') {
                        f.offset = FixedOffset::east_opt(0);
                        i += 1;
                    } else {
                        f.offset = Some(parse_offset(std::str::from_utf8(b.get(i..i + 6)?).ok()?)?);
                        i += 6;
                    }
                }
                Part::Year => f.year = Some(number(&mut i, 4)? as i32),
                Part::Month => f.month = Some(number(&mut i, 2)?),
                Part::Day => f.day = Some(number(&mut i, 2)?),
                Part::Hour => f.hour = Some(number(&mut i, 2)?),
                Part::Minute => f.minute = Some(number(&mut i, 2)?),
                Part::Second => f.second = Some(number(&mut i, 2)?),
                Part::Millis => f.millis = Some(number(&mut i, 3)?),
            }
        }
        (i == b.len()).then_some(f)
    }

    fn date_of(f: &Fields) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(f.year?, f.month?, f.day?)
    }

    fn time_of(f: &Fields) -> Option<NaiveTime> {
        // A leap second (60) is refused, as 24:00.
        NaiveTime::from_hms_milli_opt(f.hour?, f.minute?, f.second.unwrap_or(0), f.millis.unwrap_or(0))
            .filter(|_| f.second.unwrap_or(0) < 60)
    }

    pub fn read_date(&self, text: &str) -> Option<NaiveDate> {
        Pattern::date_of(&self.read(text)?)
    }

    pub fn read_time(&self, text: &str) -> Option<NaiveTime> {
        Pattern::time_of(&self.read(text)?)
    }

    /// `None` when the text does not follow the format; an error when it
    /// does, but names a wall clock the zone skips.
    pub fn read_datetime(&self, text: &str, zone: &Zone) -> Option<Result<DateTime<Utc>, String>> {
        let f = self.read(text)?;
        let wall = Pattern::date_of(&f)?.and_time(Pattern::time_of(&f)?);
        Some(match f.offset {
            Some(o) => o
                .from_local_datetime(&wall)
                .single()
                .map(|d| d.with_timezone(&Utc))
                .ok_or_else(|| format!("'{text}' is not a datetime")),
            None => zone.instant(wall).ok_or_else(|| {
                format!("'{text}' does not exist in the time zone {}: the clocks went forward then", zone.name())
            }),
        })
    }

    fn write(&self, date: Option<NaiveDate>, time: Option<NaiveTime>, offset: Option<FixedOffset>) -> String {
        let mut s = String::new();
        for part in &self.parts {
            match part {
                Part::Sep(c) => s.push(*c),
                Part::Year => s.push_str(&format!("{:04}", date.map_or(0, |d| d.year()))),
                Part::Month => s.push_str(&format!("{:02}", date.map_or(0, |d| d.month()))),
                Part::Day => s.push_str(&format!("{:02}", date.map_or(0, |d| d.day()))),
                Part::Hour => s.push_str(&format!("{:02}", time.map_or(0, |t| t.hour()))),
                Part::Minute => s.push_str(&format!("{:02}", time.map_or(0, |t| t.minute()))),
                Part::Second => s.push_str(&format!("{:02}", time.map_or(0, |t| t.second()))),
                Part::Millis => s.push_str(&format!("{:03}", time.map_or(0, |t| t.nanosecond() / 1_000_000))),
                Part::Offset => match offset {
                    Some(o) if o.local_minus_utc() == 0 => s.push('Z'),
                    Some(o) => s.push_str(&o.to_string()),
                    None => s.push('Z'),
                },
            }
        }
        s
    }

    pub fn show_date(&self, d: NaiveDate) -> String {
        self.write(Some(d), None, None)
    }

    pub fn show_time(&self, t: NaiveTime) -> String {
        self.write(None, Some(t), None)
    }

    /// An instant as a wall clock of `zone`.
    pub fn show_instant(&self, at: DateTime<Utc>, zone: &Zone) -> String {
        let wall = zone.wall(at);
        self.write(Some(wall.date()), Some(wall.time()), Some(zone.offset_at(at)))
    }
}

/// `iso` (the forms of 2.0, the default) or a format: `None` for `iso`.
pub fn parse_format(kind: FormatKind, text: &str) -> Result<Option<Pattern>, String> {
    if text.trim().eq_ignore_ascii_case("iso") || text.trim().is_empty() {
        return Ok(None);
    }
    Pattern::parse(kind, text.trim()).map(Some)
}

/// The formats of a session; `None`: ISO, as 2.0 writes and reads.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Formats {
    pub date: Option<Pattern>,
    pub time: Option<Pattern>,
    pub datetime: Option<Pattern>,
}

impl Formats {
    pub fn get(&self, kind: FormatKind) -> Option<&Pattern> {
        match kind {
            FormatKind::Date => self.date.as_ref(),
            FormatKind::Time => self.time.as_ref(),
            FormatKind::DateTime => self.datetime.as_ref(),
        }
    }

    pub fn set(&mut self, kind: FormatKind, p: Option<Pattern>) {
        match kind {
            FormatKind::Date => self.date = p,
            FormatKind::Time => self.time = p,
            FormatKind::DateTime => self.datetime = p,
        }
    }
}

thread_local! {
    /// The formats of the session running on this thread, set with its zone.
    static FORMATS: RefCell<Formats> = RefCell::new(Formats::default());
}

/// Sets the formats of the session about to run on this thread.
pub fn set_session_formats(f: Formats) {
    FORMATS.with(|x| *x.borrow_mut() = f);
}

fn session_format(kind: FormatKind) -> Option<Pattern> {
    FORMATS.with(|f| f.borrow().get(kind).cloned())
}

/// A date typed in a command: the session's format, or ISO.
pub fn read_date(text: &str) -> Result<NaiveDate, String> {
    let Some(p) = session_format(FormatKind::Date) else { return parse_date(text) };
    p.read_date(text).map(Ok).unwrap_or_else(|| {
        parse_date(text).map_err(|_| {
            format!(
                "'{text}' is not a date under the format {} of this session: write it as {} or as 2026-10-27 (ISO)",
                p.text(),
                p.show_date(NaiveDate::from_ymd_opt(2026, 10, 27).expect("a date"))
            )
        })
    })
}

/// A time typed in a command: the session's format, or ISO.
pub fn read_time(text: &str) -> Result<NaiveTime, String> {
    let Some(p) = session_format(FormatKind::Time) else { return parse_time(text) };
    p.read_time(text).map(Ok).unwrap_or_else(|| {
        parse_time(text).map_err(|_| {
            format!(
                "'{text}' is not a time under the format {} of this session: write it as {} or as 14:30:00 (ISO)",
                p.text(),
                p.show_time(NaiveTime::from_hms_opt(14, 30, 0).expect("a time"))
            )
        })
    })
}

/// A datetime typed in a command: the session's format, or ISO. Without an
/// offset, it is a wall clock of `zone`.
pub fn read_datetime(text: &str, zone: &Zone) -> Result<DateTime<Utc>, String> {
    let Some(p) = session_format(FormatKind::DateTime) else { return parse_datetime(text, zone) };
    match p.read_datetime(text, zone) {
        Some(r) => r,
        None => parse_datetime(text, zone).map_err(|_| {
            let example = NaiveDate::from_ymd_opt(2026, 10, 27).and_then(|d| d.and_hms_opt(14, 30, 0));
            format!(
                "'{text}' is not a datetime under the format {} of this session: write it as {} or as \
                 2026-10-27 14:30:00 (ISO)",
                p.text(),
                example.and_then(|w| zone.instant(w)).map(|at| p.show_instant(at, zone)).unwrap_or_default()
            )
        }),
    }
}

/// A date as the session shows it.
pub fn shown_date(d: NaiveDate) -> String {
    session_format(FormatKind::Date).map(|p| p.show_date(d)).unwrap_or_else(|| date_text(d))
}

/// A time as the session shows it.
pub fn shown_time(t: NaiveTime) -> String {
    session_format(FormatKind::Time).map(|p| p.show_time(t)).unwrap_or_else(|| time_text(t))
}

/// An instant as the session shows it, in its zone.
pub fn shown_instant(at: DateTime<Utc>) -> String {
    session_format(FormatKind::DateTime).map(|p| p.show_instant(at, &zone())).unwrap_or_else(|| show(at))
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

    #[test]
    fn formats_are_checked_when_set() {
        use FormatKind::*;
        for ok in ["DD-MM-YYYY", "MM/DD/YYYY", "DD.MM.YYYY", "YYYY-MM-DD", "YYYYMMDD", "YYYY/MM/DD"] {
            assert!(Pattern::parse(Date, ok).is_ok(), "{ok}");
        }
        for ok in ["HH:mm", "HH:mm:ss", "HH:mm:ss.SSS", "HH.mm"] {
            assert!(Pattern::parse(Time, ok).is_ok(), "{ok}");
        }
        for ok in ["DD-MM-YYYY HH:mm:ss", "YYYY-MM-DDTHH:mm:ssXXX", "DD/MM/YYYY HH:mm XXX"] {
            assert!(Pattern::parse(DateTime, ok).is_ok(), "{ok}");
        }
        let refused = [
            (Date, "YYYY-DD-MM", "month comes before the day"),
            (Date, "YY-MM-DD", "four digits"),
            (Date, "DD-MM", "YYYY is missing"),
            (Date, "DD-MM-YYYY-DD", "twice"),
            (Date, "DD-MM-YYYY HH", "has no HH"),
            (Date, "D-M-YYYY", "not one of the tokens"),
            (Date, "DD_MM_YYYY", "not one of the tokens"),
            (Time, "mm:HH", "HH, mm"),
            (Time, "HH:mm.SSS", "SSS needs ss"),
            (Time, "HH:mm XXX", "has no XXX"),
            (DateTime, "XXX DD-MM-YYYY HH:mm", "XXX comes last"),
            (DateTime, "DD-MM-YYYY", "HH is missing"),
        ];
        for (kind, text, why) in refused {
            let e = Pattern::parse(kind, text).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
        assert_eq!(parse_format(Date, "iso").unwrap(), None);
        assert_eq!(parse_format(Date, "ISO").unwrap(), None);
    }

    #[test]
    fn formats_read_strictly_and_write_back() {
        use FormatKind::*;
        let dmy = Pattern::parse(Date, "DD-MM-YYYY").unwrap();
        let mdy = Pattern::parse(Date, "MM/DD/YYYY").unwrap();
        let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(dmy.read_date("25-12-2000"), Some(day(2000, 12, 25)));
        assert_eq!(dmy.read_date(" 25-12-2000 "), Some(day(2000, 12, 25)));
        assert_eq!(dmy.read_date("29-02-2028"), Some(day(2028, 2, 29)), "a leap year");
        for wrong in ["29-02-2027", "31-04-2026", "00-01-2026", "32-01-2026", "01-13-2026", "1-1-2026", "25/12/2000"] {
            assert_eq!(dmy.read_date(wrong), None, "{wrong}");
        }
        // Same digits, two days: the format decides, nothing is guessed.
        assert_eq!(dmy.read_date("05-06-2026"), Some(day(2026, 6, 5)));
        assert_eq!(mdy.read_date("05/06/2026"), Some(day(2026, 5, 6)));
        assert_eq!(dmy.show_date(day(2026, 6, 5)), "05-06-2026");
        assert_eq!(mdy.show_date(day(2026, 6, 5)), "06/05/2026");
        assert_eq!(dmy.show_date(day(1, 1, 1)), "01-01-0001");

        let hm = Pattern::parse(Time, "HH:mm").unwrap();
        let ms = Pattern::parse(Time, "HH:mm:ss.SSS").unwrap();
        assert_eq!(hm.read_time("08:30"), NaiveTime::from_hms_opt(8, 30, 0));
        assert_eq!(hm.read_time("08:30:00"), None);
        assert_eq!(hm.read_time("24:00"), None);
        assert_eq!(ms.read_time("23:59:60.000"), None, "no leap second");
        assert_eq!(ms.read_time("08:30:15.250"), NaiveTime::from_hms_milli_opt(8, 30, 15, 250));
        assert_eq!(ms.show_time(NaiveTime::from_hms_milli_opt(8, 30, 15, 250).unwrap()), "08:30:15.250");
        assert_eq!(hm.show_time(NaiveTime::from_hms_opt(8, 30, 15).unwrap()), "08:30");

        let dt = Pattern::parse(DateTime, "DD/MM/YYYY HH:mm XXX").unwrap();
        let kin = Zone::parse("Africa/Kinshasa").unwrap();
        let utc = Zone::Utc;
        let at = dt.read_datetime("31/10/2026 16:18 +01:00", &utc).unwrap().unwrap();
        assert_eq!(iso(at), "2026-10-31T15:18:00.000Z");
        assert_eq!(dt.read_datetime("31/10/2026 15:18 Z", &kin).unwrap().unwrap(), at);
        // Without its offset, a wall clock of the zone that reads it.
        assert_eq!(dt.read_datetime("31/10/2026 16:18", &kin).unwrap().unwrap(), at);
        assert_eq!(iso(dt.read_datetime("31/10/2026 16:18", &utc).unwrap().unwrap()), "2026-10-31T16:18:00.000Z");
        assert!(dt.read_datetime("31/10/2026 16:18 +15:00", &utc).is_none());
        assert_eq!(dt.show_instant(at, &kin), "31/10/2026 16:18 +01:00");
        assert_eq!(dt.show_instant(at, &utc), "31/10/2026 15:18 Z");
        // The hour skipped in spring does not exist; the hour repeated in autumn is read as the first.
        let ny = Zone::parse("America/New_York").unwrap();
        let e = dt.read_datetime("08/03/2026 02:30", &ny).unwrap().unwrap_err();
        assert!(e.contains("does not exist"), "{e}");
        let first = dt.read_datetime("01/11/2026 01:30", &ny).unwrap().unwrap();
        assert_eq!(iso(first), "2026-11-01T05:30:00.000Z");
    }

    #[test]
    fn a_session_reads_its_format_and_iso_and_shows_its_format() {
        let mut f = Formats::default();
        f.set(FormatKind::Date, parse_format(FormatKind::Date, "DD-MM-YYYY").unwrap());
        set_session_formats(f);
        let day = NaiveDate::from_ymd_opt(2000, 12, 25).unwrap();
        assert_eq!(read_date("25-12-2000").unwrap(), day);
        assert_eq!(read_date("2000-12-25").unwrap(), day, "ISO is always accepted");
        let e = read_date("12/25/2000").unwrap_err();
        assert!(e.contains("DD-MM-YYYY") && e.contains("2026-10-27 (ISO)"), "{e}");
        assert_eq!(shown_date(day), "25-12-2000");
        assert_eq!(shown_time(NaiveTime::from_hms_opt(8, 0, 0).unwrap()), "08:00:00", "no time format: ISO");
        set_session_formats(Formats::default());
        assert_eq!(shown_date(day), "2000-12-25");
        assert!(read_date("25-12-2000").is_err());
    }
}
