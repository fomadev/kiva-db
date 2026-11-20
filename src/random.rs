//! Random values and UUIDs: `random()`, `rand()`, `random_int(min, max)`,
//! `random_string(n)`, `random_bytes(n)`, `uuid()` and `uuid_v7()`
//! (docs/architecture/RANDOM_FUNCTIONS.md, lot R1).
//!
//! Every value comes from the operating system's generator
//! ([`crate::util::fill_random`]), even `random()`: no function is weak by
//! mistake. When it fails, the draw fails with `ERR_RANDOM_UNAVAILABLE`, and
//! nothing takes its place. A function is evaluated once, where a command is
//! read, and only its value is stored: replicas, the change journal and
//! backups never draw again.

use crate::util::fill_random;
use std::sync::Mutex;

/// The names a command, a document, `print` or a schema default may call.
pub const NAMES: [&str; 7] = ["random", "rand", "random_int", "random_string", "random_bytes", "uuid", "uuid_v7"];

/// The bounds of `random_string(n)` and `random_bytes(n)`.
pub const MAX_LENGTH: usize = 1024;

const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// The functions of arrays (lot R2): their first argument is an array.
pub const ARRAY_NAMES: [&str; 2] = ["shuffle", "sample"];

/// Whether `word` names one of the functions, of values or of arrays, in any case.
pub fn is_name(word: &str) -> bool {
    NAMES.iter().chain(ARRAY_NAMES.iter()).any(|n| n.eq_ignore_ascii_case(word))
}

/// Whether `word` names `shuffle` or `sample`.
pub fn is_array_name(word: &str) -> bool {
    ARRAY_NAMES.iter().any(|n| n.eq_ignore_ascii_case(word))
}

/// `shuffle(array)`, `sample(array)` or `sample(array, n)`, read but not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrayFn {
    Shuffle,
    /// One element, not an array.
    SampleOne,
    /// `n` elements at distinct positions, as an array.
    Sample(usize),
}

/// What an [`ArrayFn`] gives: an element, or an array.
#[derive(Debug, PartialEq)]
pub enum Picked<T> {
    One(T),
    Many(Vec<T>),
}

impl ArrayFn {
    /// Reads the call from its name and the text of its count, if any.
    pub fn parse(name: &str, count: Option<&str>) -> Result<ArrayFn, String> {
        match (name.to_ascii_lowercase().as_str(), count) {
            ("shuffle", None) => Ok(ArrayFn::Shuffle),
            ("shuffle", Some(_)) => Err("shuffle takes one array: shuffle([\"A\", \"B\", \"C\"])".into()),
            ("sample", None) => Ok(ArrayFn::SampleOne),
            ("sample", Some(t)) => {
                let t = t.trim();
                t.parse::<usize>()
                    .map(ArrayFn::Sample)
                    .map_err(|_| format!("sample: the count '{t}' is not a whole number of 0 or more"))
            }
            (other, _) => Err(format!("'{other}' is not a function of arrays: shuffle, sample")),
        }
    }

    /// The name, for messages.
    pub fn name(self) -> &'static str {
        match self {
            ArrayFn::Shuffle => "shuffle",
            _ => "sample",
        }
    }

    /// Applies the call to the elements of an array, which it takes.
    ///
    /// - `shuffle`: Fisher–Yates, every order equally likely;
    /// - `sample(a, n)`: the first `n` steps of Fisher–Yates, then the rest
    ///   is dropped: `n` distinct positions, in random order, `n` draws;
    /// - `sample(a)`: one position.
    ///
    /// The array is reordered in place: nothing is copied.
    pub fn apply<T>(self, mut items: Vec<T>, src: &mut Source) -> Result<Picked<T>, String> {
        let len = items.len();
        let draw = |e: crate::KivaError| e.to_string();
        match self {
            ArrayFn::Shuffle => {
                partial_shuffle(&mut items, len, src).map_err(draw)?;
                Ok(Picked::Many(items))
            }
            ArrayFn::SampleOne => {
                if len == 0 {
                    return Err("sample: an empty array has no element to give".into());
                }
                let i = src.below(len as u64).map_err(draw)? as usize;
                Ok(Picked::One(items.swap_remove(i)))
            }
            ArrayFn::Sample(n) => {
                if n > len {
                    return Err(format!(
                        "sample: {n} element{} asked of an array of {len}",
                        if n == 1 { "" } else { "s" }
                    ));
                }
                partial_shuffle(&mut items, n, src).map_err(draw)?;
                items.truncate(n);
                Ok(Picked::Many(items))
            }
        }
    }
}

/// Puts `n` elements drawn uniformly without repetition at the front of
/// `items`, in random order (the first `n` steps of Fisher–Yates). With
/// `n` = the length, the whole array is shuffled.
pub fn partial_shuffle<T>(items: &mut [T], n: usize, src: &mut Source) -> crate::Result<()> {
    let len = items.len();
    for i in 0..n.min(len.saturating_sub(1)) {
        let j = i + src.below((len - i) as u64)? as usize;
        items.swap(i, j);
    }
    Ok(())
}

/// A call, read and checked, not yet drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gen {
    /// `random()` or `rand()`: a float in [0, 1).
    Random,
    /// `random_int(min, max)`, bounds included.
    Int(i64, i64),
    /// `random_string(n)`: `n` characters of `A-Z a-z 0-9`.
    Str(usize),
    /// `random_bytes(n)`: `n` bytes, as base64url without padding.
    Bytes(usize),
    /// `uuid()`: a UUID of version 4.
    Uuid,
    /// `uuid_v7()`: a UUID of version 7, increasing within the process.
    UuidV7,
}

/// A value drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Drawn {
    Float(f64),
    Int(i64),
    Text(String),
}

impl Drawn {
    /// A float always shows a fraction, so it is read back as a float: `0.0`, not `0`.
    pub fn float_text(f: f64) -> String {
        let s = f.to_string();
        if s.contains(['.', 'e', 'E']) {
            s
        } else {
            format!("{s}.0")
        }
    }
}

impl Gen {
    /// Reads a call from its name and the texts of its arguments.
    pub fn parse(name: &str, args: &[String]) -> Result<Gen, String> {
        let name = name.to_ascii_lowercase();
        let count = |n: usize, usage: &str| {
            if args.len() == n {
                Ok(())
            } else {
                Err(format!("{name}() takes {}: {usage}", if n == 0 { "no argument".into() } else { format!("{n}") }))
            }
        };
        match name.as_str() {
            "random" | "rand" => count(0, &format!("{name}()")).map(|_| Gen::Random),
            "uuid" => count(0, "uuid()").map(|_| Gen::Uuid),
            "uuid_v7" => count(0, "uuid_v7()").map(|_| Gen::UuidV7),
            "random_int" => {
                count(2, "random_int(1, 6)")?;
                let bound = |t: &str, which: &str| {
                    t.trim().parse::<i64>().map_err(|_| {
                        format!(
                            "random_int: the {which} bound '{t}' is not a whole number between \
                             -9223372036854775808 and 9223372036854775807"
                        )
                    })
                };
                let (min, max) = (bound(&args[0], "lower")?, bound(&args[1], "upper")?);
                if min > max {
                    return Err(format!("random_int({min}, {max}): the lower bound is above the upper bound"));
                }
                Ok(Gen::Int(min, max))
            }
            "random_string" | "random_bytes" => {
                count(1, &format!("{name}(16)"))?;
                let t = args[0].trim();
                let n =
                    t.parse::<usize>().ok().filter(|n| (1..=MAX_LENGTH).contains(n)).ok_or_else(|| {
                        format!("{name}: the length '{t}' is not a whole number from 1 to {MAX_LENGTH}")
                    })?;
                Ok(if name == "random_string" { Gen::Str(n) } else { Gen::Bytes(n) })
            }
            "shuffle" | "sample" => {
                Err(format!("{name} works on an array, in a command or a document: {name}([1, 2, 3])"))
            }
            _ => Err(format!("'{name}' is not a random function: {}", NAMES.join(", "))),
        }
    }

    /// A call written as a text, `"uuid_v7()"` or `"random_int(1, 6)"`, as a
    /// schema default writes it: `None` when the text is not a call of one
    /// of the functions, so that it stays a plain text.
    pub fn from_text(text: &str) -> Option<Result<Gen, String>> {
        let t = text.trim();
        let open = t.find('(')?;
        let name = t[..open].trim();
        if !is_name(name) || !t.ends_with(')') {
            return None;
        }
        let inside = t[open + 1..t.len() - 1].trim();
        let args: Vec<String> =
            if inside.is_empty() { Vec::new() } else { inside.split(',').map(|a| a.trim().to_string()).collect() };
        Some(Gen::parse(name, &args))
    }

    /// The call as it is written: `random_int(1, 6)`.
    pub fn call(&self) -> String {
        match self {
            Gen::Random => "random()".into(),
            Gen::Int(a, b) => format!("random_int({a}, {b})"),
            Gen::Str(n) => format!("random_string({n})"),
            Gen::Bytes(n) => format!("random_bytes({n})"),
            Gen::Uuid => "uuid()".into(),
            Gen::UuidV7 => "uuid_v7()".into(),
        }
    }

    /// Draws a value.
    pub fn draw(&self) -> crate::Result<Drawn> {
        self.draw_from(&mut Source::new())
    }

    /// Draws a value from `src`, so that the draws of one command share its bytes.
    pub fn draw_from(&self, src: &mut Source) -> crate::Result<Drawn> {
        Ok(match *self {
            Gen::Random => Drawn::Float(src.float()?),
            Gen::Int(min, max) => Drawn::Int(src.int_between(min, max)?),
            Gen::Str(n) => Drawn::Text(src.string(n)?),
            Gen::Bytes(n) => Drawn::Text(base64url(&src.bytes(n)?)),
            Gen::Uuid => Drawn::Text(uuid_v4(src)?),
            Gen::UuidV7 => Drawn::Text(uuid_v7(src)?),
        })
    }
}

/// Random bytes for one command: drawn from the operating system in blocks,
/// so that many draws make few system calls.
pub struct Source {
    buf: [u8; 256],
    at: usize,
}

impl Default for Source {
    fn default() -> Self {
        Self::new()
    }
}

impl Source {
    pub fn new() -> Source {
        Source { buf: [0; 256], at: 256 }
    }

    fn take(&mut self, out: &mut [u8]) -> crate::Result<()> {
        if out.len() > self.buf.len() {
            return fill_random(out);
        }
        if self.at + out.len() > self.buf.len() {
            fill_random(&mut self.buf)?;
            self.at = 0;
        }
        out.copy_from_slice(&self.buf[self.at..self.at + out.len()]);
        // Bytes given are not kept where they could be given twice.
        self.buf[self.at..self.at + out.len()].fill(0);
        self.at += out.len();
        Ok(())
    }

    pub fn u64(&mut self) -> crate::Result<u64> {
        let mut b = [0u8; 8];
        self.take(&mut b)?;
        Ok(u64::from_le_bytes(b))
    }

    fn byte(&mut self) -> crate::Result<u8> {
        let mut b = [0u8; 1];
        self.take(&mut b)?;
        Ok(b[0])
    }

    pub fn bytes(&mut self, n: usize) -> crate::Result<Vec<u8>> {
        let mut v = vec![0u8; n];
        self.take(&mut v)?;
        Ok(v)
    }

    /// A float in [0, 1): 53 random bits, every multiple of 2^-53 equally likely.
    pub fn float(&mut self) -> crate::Result<f64> {
        Ok((self.u64()? >> 11) as f64 * (1.0 / (1u64 << 53) as f64))
    }

    /// An integer in [0, n), `n` > 0, without bias: draws that would favour
    /// the small values are rejected, never folded by `%`.
    pub fn below(&mut self, n: u64) -> crate::Result<u64> {
        debug_assert!(n > 0);
        // 2^64 mod n: the draws below it are the ones `%` would make uneven.
        let skip = n.wrapping_neg() % n;
        loop {
            let x = self.u64()?;
            if x >= skip {
                return Ok(x % n);
            }
        }
    }

    /// An integer in [min, max], bounds included; the span may be 2^64.
    pub fn int_between(&mut self, min: i64, max: i64) -> crate::Result<i64> {
        let span = (max as i128 - min as i128 + 1) as u128;
        let offset = if span > u64::MAX as u128 { self.u64()? } else { self.below(span as u64)? };
        Ok((min as i128 + offset as i128) as i64)
    }

    /// `n` characters of `A-Z a-z 0-9`, each by rejection: a byte of 248 or
    /// more (62 × 4) is drawn again.
    pub fn string(&mut self, n: usize) -> crate::Result<String> {
        let mut s = String::with_capacity(n);
        while s.len() < n {
            let b = self.byte()?;
            if b < 248 {
                s.push(ALPHABET[(b % 62) as usize] as char);
            }
        }
        Ok(s)
    }
}

/// Bytes as base64url without padding (RFC 4648, 5).
pub fn base64url(bytes: &[u8]) -> String {
    crate::util::b64_encode(bytes)
        .trim_end_matches('=')
        .chars()
        .map(|c| match c {
            '+' => '-',
            '/' => '_',
            c => c,
        })
        .collect()
}

fn uuid_text(b: &[u8; 16]) -> String {
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Sets the variant bits of RFC 9562 (`10`) in byte 8.
fn variant(b: &mut [u8; 16]) {
    b[8] = (b[8] & 0x3f) | 0x80;
}

/// A UUID of version 4: 122 random bits.
pub fn uuid_v4(src: &mut Source) -> crate::Result<String> {
    let mut b = [0u8; 16];
    src.take(&mut b)?;
    b[6] = (b[6] & 0x0f) | 0x40;
    variant(&mut b);
    Ok(uuid_text(&b))
}

/// The last UUID v7 of the process: its milliseconds and its counter.
#[derive(Clone, Copy, Debug, Default)]
pub struct V7 {
    ms: u64,
    counter: u16,
}

static LAST_V7: Mutex<V7> = Mutex::new(V7 { ms: 0, counter: 0 });

/// A UUID of version 7 (RFC 9562, method 1), strictly increasing within the process.
pub fn uuid_v7(src: &mut Source) -> crate::Result<String> {
    let now = crate::time::now().timestamp_millis().max(0) as u64;
    let mut last = LAST_V7.lock().unwrap_or_else(|e| e.into_inner());
    v7_next(&mut last, now, src)
}

/// The next UUID v7 after `last`, at `now` milliseconds:
///
/// - in a new millisecond, the 12-bit counter of `rand_a` starts from a
///   random value below 2048, which leaves room to count;
/// - in the same millisecond, or when the clock went back, the previous
///   timestamp is kept and the counter goes up by one;
/// - when the counter is full, the timestamp moves 1 ms forward, ahead of the
///   clock, and the counter starts again.
pub fn v7_next(last: &mut V7, now: u64, src: &mut Source) -> crate::Result<String> {
    let mut b = [0u8; 16];
    src.take(&mut b[6..16])?;
    let fresh = u16::from_be_bytes([b[6], b[7]]) & 0x07ff;
    let mut next = *last;
    if now > next.ms {
        next = V7 { ms: now, counter: fresh };
    } else if next.counter < 0x0fff {
        next.counter += 1;
    } else {
        next = V7 { ms: next.ms + 1, counter: fresh };
    }
    let ms = next.ms & 0xffff_ffff_ffff;
    b[0..6].copy_from_slice(&ms.to_be_bytes()[2..8]);
    b[6] = 0x70 | (next.counter >> 8) as u8;
    b[7] = next.counter as u8;
    variant(&mut b);
    // Only once every byte is drawn: a failed draw leaves the state as it was.
    *last = next;
    Ok(uuid_text(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::tests::with_failing_random;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn hex_bytes(u: &str) -> Vec<u8> {
        let h: String = u.chars().filter(|c| *c != '-').collect();
        (0..16).map(|i| u8::from_str_radix(&h[2 * i..2 * i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn calls_are_read_and_checked() {
        assert_eq!(Gen::parse("RAND", &[]).unwrap(), Gen::Random);
        assert_eq!(Gen::parse("random_int", &args(&["-5", " 5"])).unwrap(), Gen::Int(-5, 5));
        assert_eq!(Gen::parse("random_string", &args(&["1024"])).unwrap(), Gen::Str(1024));
        assert_eq!(Gen::parse("random_bytes", &args(&["1"])).unwrap(), Gen::Bytes(1));
        for (name, a, says) in [
            ("random_int", vec!["5", "1"], "lower bound is above"),
            ("random_int", vec!["1.5", "2"], "not a whole number"),
            ("random_int", vec!["1", "9223372036854775808"], "not a whole number"),
            ("random_int", vec!["1"], "takes 2"),
            ("random_string", vec!["0"], "from 1 to 1024"),
            ("random_string", vec!["1025"], "from 1 to 1024"),
            ("random_bytes", vec!["-1"], "from 1 to 1024"),
            ("uuid", vec!["1"], "no argument"),
            ("random", vec!["1"], "no argument"),
        ] {
            let e = Gen::parse(name, &args(&a)).unwrap_err();
            assert!(e.contains(says), "{name}{a:?}: {e}");
        }
    }

    #[test]
    fn integers_stay_within_their_bounds() {
        let mut s = Source::new();
        for _ in 0..1000 {
            assert!((1..=6).contains(&s.int_between(1, 6).unwrap()));
            assert_eq!(s.int_between(7, 7).unwrap(), 7);
            assert!((-1..=0).contains(&s.int_between(-1, 0).unwrap()));
        }
        // The widest span, 2^64, and the ends of i64.
        let (lo, hi) = (i64::MIN, i64::MAX);
        let mut signs = (false, false);
        for _ in 0..200 {
            let v = s.int_between(lo, hi).unwrap();
            signs = (signs.0 || v < 0, signs.1 || v >= 0);
        }
        assert_eq!(signs, (true, true));
        assert_eq!(s.int_between(hi, hi).unwrap(), hi);
        assert_eq!(s.int_between(lo, lo).unwrap(), lo);
        assert!(s.int_between(hi - 1, hi).unwrap() >= hi - 1);
    }

    #[test]
    fn a_die_is_fair_enough() {
        // Chi-square with 5 degrees of freedom: 20.5 is passed by chance once in
        // a thousand runs; a die favouring one face by 10 % fails it.
        let mut s = Source::new();
        let n = 60_000;
        let mut count = [0f64; 6];
        for _ in 0..n {
            count[(s.int_between(1, 6).unwrap() - 1) as usize] += 1.0;
        }
        let e = n as f64 / 6.0;
        let chi: f64 = count.iter().map(|c| (c - e) * (c - e) / e).sum();
        assert!(chi < 20.5, "chi-square {chi}: {count:?}");
    }

    #[test]
    fn floats_are_in_zero_one() {
        let mut s = Source::new();
        let mut sum = 0.0;
        for _ in 0..10_000 {
            let f = s.float().unwrap();
            assert!((0.0..1.0).contains(&f));
            sum += f;
        }
        // The mean of 10 000 uniform draws is 0.5 ± 0.0029 (one standard deviation).
        assert!((sum / 10_000.0 - 0.5).abs() < 0.02, "{sum}");
        assert_eq!(Drawn::float_text(0.0), "0.0");
        assert_eq!(Drawn::float_text(0.25), "0.25");
        assert_eq!(Drawn::float_text(2f64.powi(-53)), format!("{}", 2f64.powi(-53)));
    }

    #[test]
    fn strings_and_bytes() {
        let mut s = Source::new();
        let t = s.string(1024).unwrap();
        assert_eq!(t.len(), 1024);
        assert!(t.bytes().all(|c| c.is_ascii_alphanumeric()));
        // All 62 symbols come up in 1024 characters (each misses with odds of about e^-16).
        let seen: std::collections::HashSet<u8> = t.bytes().collect();
        assert_eq!(seen.len(), 62);
        assert_ne!(s.string(16).unwrap(), s.string(16).unwrap());
        assert_eq!(base64url(&[0xfb, 0xff]), "-_8");
        assert_eq!(base64url(b"any carnal pleas"), "YW55IGNhcm5hbCBwbGVhcw");
        let Drawn::Text(b) = Gen::Bytes(32).draw().unwrap() else { panic!() };
        assert_eq!(b.len(), 43);
        assert!(b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        // A draw larger than the block of the source.
        assert_eq!(s.bytes(1024).unwrap().len(), 1024);
    }

    #[test]
    fn uuid_v4_has_its_version_and_variant() {
        let mut s = Source::new();
        let a = uuid_v4(&mut s).unwrap();
        assert_eq!(a.len(), 36);
        assert_eq!(crate::literal::uuid(&a).unwrap(), a, "a uuid the literal of C4 accepts");
        let b = hex_bytes(&a);
        assert_eq!(b[6] >> 4, 4);
        assert_eq!(b[8] >> 6, 0b10);
        assert_ne!(a, uuid_v4(&mut s).unwrap());
    }

    #[test]
    fn uuid_v7_carries_the_time_and_increases() {
        let mut s = Source::new();
        let mut last = V7::default();
        let at = 1_790_000_000_123u64;
        let a = v7_next(&mut last, at, &mut s).unwrap();
        let b = hex_bytes(&a);
        assert_eq!(b[6] >> 4, 7);
        assert_eq!(b[8] >> 6, 0b10);
        let mut ms = [0u8; 8];
        ms[2..].copy_from_slice(&b[0..6]);
        assert_eq!(u64::from_be_bytes(ms), at);
        assert!(last.counter < 2048, "a new millisecond starts low");
        // The same millisecond, then a clock gone back: still increasing.
        let mut prev = a;
        for now in [at, at, at - 5_000, at] {
            let next = v7_next(&mut last, now, &mut s).unwrap();
            assert!(next > prev, "{next} after {prev}");
            assert_eq!(last.ms, at);
            prev = next;
        }
        // A full counter moves the timestamp 1 ms forward.
        last.counter = 0x0fff;
        let next = v7_next(&mut last, at, &mut s).unwrap();
        assert!(next > prev);
        assert_eq!(last.ms, at + 1);
        // 10 000 in one millisecond stay strictly increasing and distinct.
        let mut prev = next;
        for _ in 0..10_000 {
            let next = v7_next(&mut last, at, &mut s).unwrap();
            assert!(next > prev);
            prev = next;
        }
        // A later clock takes over again.
        let later = v7_next(&mut last, at + 60_000, &mut s).unwrap();
        assert!(later > prev);
        assert_eq!(last.ms, at + 60_000);
    }

    #[test]
    fn the_process_wide_uuid_v7_increases() {
        let mut s = Source::new();
        let mut prev = uuid_v7(&mut s).unwrap();
        for _ in 0..2_000 {
            let next = uuid_v7(&mut s).unwrap();
            assert!(next > prev);
            prev = next;
        }
    }

    #[test]
    fn array_calls_are_read_and_checked() {
        assert_eq!(ArrayFn::parse("SHUFFLE", None).unwrap(), ArrayFn::Shuffle);
        assert_eq!(ArrayFn::parse("sample", None).unwrap(), ArrayFn::SampleOne);
        assert_eq!(ArrayFn::parse("sample", Some(" 3")).unwrap(), ArrayFn::Sample(3));
        assert!(ArrayFn::parse("shuffle", Some("2")).unwrap_err().contains("one array"));
        assert!(ArrayFn::parse("sample", Some("-1")).unwrap_err().contains("0 or more"));
        assert!(ArrayFn::parse("sample", Some("1.5")).unwrap_err().contains("0 or more"));
        assert!(Gen::parse("shuffle", &[]).unwrap_err().contains("works on an array"));
        let mut s = Source::new();
        assert_eq!(ArrayFn::Shuffle.apply(Vec::<u8>::new(), &mut s).unwrap(), Picked::Many(vec![]));
        assert_eq!(ArrayFn::Shuffle.apply(vec![7], &mut s).unwrap(), Picked::Many(vec![7]));
        assert_eq!(ArrayFn::Sample(0).apply(vec![1, 2], &mut s).unwrap(), Picked::Many(vec![]));
        assert_eq!(ArrayFn::SampleOne.apply(vec![9], &mut s).unwrap(), Picked::One(9));
        assert!(ArrayFn::SampleOne.apply(Vec::<u8>::new(), &mut s).unwrap_err().contains("empty array"));
        assert!(ArrayFn::Sample(3)
            .apply(vec![1, 2], &mut s)
            .unwrap_err()
            .contains("3 elements asked of an array of 2"));
    }

    #[test]
    fn shuffles_keep_every_element() {
        let mut s = Source::new();
        // Duplicates included, and an array far larger than the block of the source.
        let items: Vec<u32> = (0..100_000).map(|i| i % 50_000).collect();
        let Picked::Many(mut out) = ArrayFn::Shuffle.apply(items.clone(), &mut s).unwrap() else { panic!() };
        assert_ne!(out, items, "100 000 elements left in order: practically impossible");
        out.sort_unstable();
        let mut want = items;
        want.sort_unstable();
        assert_eq!(out, want);
        // A sample takes distinct positions: equal values only as often as the array holds them.
        let Picked::Many(mut few) = ArrayFn::Sample(5).apply((0..10).collect::<Vec<u32>>(), &mut s).unwrap() else {
            panic!()
        };
        assert_eq!(few.len(), 5);
        few.sort_unstable();
        few.dedup();
        assert_eq!(few.len(), 5);
        let Picked::Many(all) = ArrayFn::Sample(3).apply(vec!["a", "a", "b"], &mut s).unwrap() else { panic!() };
        assert_eq!(all.iter().filter(|x| **x == "a").count(), 2);
    }

    #[test]
    fn every_order_of_four_is_as_likely() {
        // 24 orders, 48 000 shuffles: chi-square with 23 degrees of freedom,
        // 49.7 is passed by chance once in a thousand runs.
        let mut s = Source::new();
        let mut count = std::collections::HashMap::new();
        for _ in 0..48_000 {
            let Picked::Many(v) = ArrayFn::Shuffle.apply(vec![0u8, 1, 2, 3], &mut s).unwrap() else { panic!() };
            *count.entry(v).or_insert(0f64) += 1.0;
        }
        assert_eq!(count.len(), 24);
        let chi: f64 = count.values().map(|c| (c - 2000.0) * (c - 2000.0) / 2000.0).sum();
        assert!(chi < 49.7, "chi-square {chi}");
    }

    #[test]
    fn samples_take_each_element_as_often() {
        // sample(10 elements, 3), 30 000 times: each element 9 000 times
        // expected; chi-square with 9 degrees of freedom, 27.9 at p = 0.001.
        // sample(a) alone: each of 5 elements 2 000 times of 10 000; 18.5 at 4 degrees.
        let mut s = Source::new();
        let mut count = [0f64; 10];
        for _ in 0..30_000 {
            let Picked::Many(v) = ArrayFn::Sample(3).apply((0..10).collect::<Vec<usize>>(), &mut s).unwrap() else {
                panic!()
            };
            for i in v {
                count[i] += 1.0;
            }
        }
        let chi: f64 = count.iter().map(|c| (c - 9000.0) * (c - 9000.0) / 9000.0).sum();
        assert!(chi < 27.9, "chi-square {chi}: {count:?}");
        let mut one = [0f64; 5];
        for _ in 0..10_000 {
            let Picked::One(i) = ArrayFn::SampleOne.apply((0..5).collect::<Vec<usize>>(), &mut s).unwrap() else {
                panic!()
            };
            one[i] += 1.0;
        }
        let chi: f64 = one.iter().map(|c| (c - 2000.0) * (c - 2000.0) / 2000.0).sum();
        assert!(chi < 18.5, "chi-square {chi}: {one:?}");
    }

    #[test]
    fn a_broken_generator_fails_shuffles_and_samples() {
        for call in [ArrayFn::Shuffle, ArrayFn::SampleOne, ArrayFn::Sample(2)] {
            let e = with_failing_random(|| call.apply(vec![1, 2, 3], &mut Source::new())).unwrap_err();
            assert!(e.starts_with("ERR_RANDOM_UNAVAILABLE"), "{e}");
        }
    }

    #[test]
    fn a_broken_generator_fails_every_function() {
        for g in [Gen::Random, Gen::Int(1, 6), Gen::Str(8), Gen::Bytes(8), Gen::Uuid, Gen::UuidV7] {
            let e = with_failing_random(|| g.draw()).unwrap_err();
            assert!(e.to_string().starts_with("ERR_RANDOM_UNAVAILABLE"), "{}: {e}", g.call());
        }
        // A command fails, and writes nothing: a value, a document, a schema
        // default, a script.
        let dir = std::env::temp_dir().join(format!("kivadb_random_broken_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let inst = crate::instance::Instance::open(&dir, crate::instance::Options::default()).unwrap();
        let mut s = crate::session::Session::new(inst.clone(), true, None);
        assert!(!s.execute(r#"schema set d:* {"id": {"type": "string", "default": "uuid()"}}"#).has_err());
        for cmd in [
            "set k uuid()",
            "set k random_int(1, 6)",
            r#"set k {"a": random()}"#,
            "set k shuffle([1, 2, 3])",
            r#"set k {"a": sample([1, 2, 3], 2)}"#,
            r#"eval 'kiva.set("k", kiva.shuffle({1, 2})) return 1'"#,
            r#"set d:1 {"name": "x"}"#,
            r#"eval 'kiva.set("k", kiva.random_string(4)) return 1'"#,
        ] {
            let out = with_failing_random(|| s.execute(cmd));
            assert!(out.has_err() && out.text().contains("ERR_RANDOM_UNAVAILABLE"), "{cmd}: {}", out.text());
        }
        let e = inst.engine("default").unwrap();
        assert!(e.get("k").unwrap().is_none() && e.get("d:1").unwrap().is_none());
        drop((s, e, inst));
        let _ = std::fs::remove_dir_all(&dir);
        // A failed uuid_v7 leaves the state as it was.
        let mut last = V7 { ms: 10, counter: 5 };
        assert!(with_failing_random(|| v7_next(&mut last, 10, &mut Source::new())).is_err());
        assert_eq!((last.ms, last.counter), (10, 5));
    }
}
