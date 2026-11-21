// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Lot P0 of docs/architecture/RC5_ARCHITECTURE.md (1.6 and 4.1): a pattern
//! without a wildcard names one key, which a query reads alone (`KEY
//! LOOKUP`), and the shell says how to reach the keys of a family when that
//! key holds nothing. And every usage text is written with one `Usage:`.

mod common;

use common::{fails, instance, ok, run, TempDir};
use kivadb::session::Session;

fn keys_examined(explain: &str) -> u64 {
    explain
        .lines()
        .find_map(|l| l.trim().strip_prefix("Keys examined :"))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or_else(|| panic!("no 'Keys examined' in:\n{explain}"))
}

#[test]
fn an_exact_key_is_read_alone_and_the_results_do_not_change() {
    let dir = TempDir::new("exact_keys");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, r#"set user {"city": "Kinshasa", "n": 1}"#);
    for i in 0..50 {
        ok(&mut s, &format!(r#"set user:{i} {{"city": "Kinshasa", "n": {i}}}"#));
        ok(&mut s, &format!(r#"set userx:{i} {{"city": "Kinshasa"}}"#));
    }

    // The same rows as before: the key `user` alone.
    let rows = ok(&mut s, r#"get where user.city = "Kinshasa""#);
    assert!(rows.starts_with("user: ") && rows.contains("(1 matches)"), "{rows}");
    assert_eq!(ok(&mut s, r#"count where user.city = "Kinshasa""#), "Count: 1");
    assert!(ok(&mut s, r#"get where user.n > 5"#).contains("(0 matches)"));
    // But one key is read, not the 101 that start with `user`.
    let plan = ok(&mut s, r#"explain analyze get where user.city = "Kinshasa""#);
    assert!(plan.contains("Strategy  : KEY LOOKUP"), "{plan}");
    assert!(plan.contains("Estimated : 1 of 101 keys examined"), "{plan}");
    assert_eq!(keys_examined(&plan), 1, "{plan}");
    assert!(!plan.contains("Tip"), "no index helps one key:\n{plan}");
    // With an order, a limit, `return`, or an index on the family: still one key.
    let ordered = ok(&mut s, r#"explain analyze get where user.n >= 0 order by n desc limit 5"#);
    assert!(ordered.contains("KEY LOOKUP") && !ordered.contains("Tip"), "{ordered}");
    assert_eq!(keys_examined(&ordered), 1, "{ordered}");
    ok(&mut s, "create index on user:* (city)");
    let indexed = ok(&mut s, r#"explain analyze get where user.city = "Kinshasa""#);
    assert!(indexed.contains("KEY LOOKUP"), "{indexed}");
    assert_eq!(keys_examined(&indexed), 1, "{indexed}");
    assert!(ok(&mut s, r#"get where user.city = "Kinshasa" return n"#).contains(r#"user: {"n":1}"#));

    // The family, with its index, as before.
    assert_eq!(ok(&mut s, r#"count where user:*.city = "Kinshasa""#), "Count: 50");
    assert!(ok(&mut s, r#"explain get where user:*.city = "Kinshasa""#).contains("INDEX SCAN"));

    // An exact key that holds nothing, next to a family that does: a hint.
    ok(&mut s, "del user");
    let missing = ok(&mut s, r#"get where user.city = "Kinshasa""#);
    assert!(missing.contains("(0 matches)"), "{missing}");
    assert!(missing.contains("there is no key 'user'") && missing.contains("write user:*"), "{missing}");
    let plan = ok(&mut s, r#"explain analyze get where user.city = "Kinshasa""#);
    assert!(plan.contains("Estimated : 0 of 100 keys examined"), "{plan}");
    assert_eq!(keys_examined(&plan), 0, "{plan}");
    // No family, no hint.
    let none = ok(&mut s, r#"get where nobody.city = "Goma""#);
    assert_eq!(none.trim(), "(0 matches)");
    // A key that exists but does not match: no hint either.
    ok(&mut s, r#"set place {"city": "Goma"} and place:1 {"city": "Goma"}"#);
    assert_eq!(ok(&mut s, r#"get where place.city = "Bukavu""#).trim(), "(0 matches)");
}

#[test]
fn every_usage_says_usage_once() {
    let dir = TempDir::new("usage_once");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, r#"set k {"a": 1}"#);
    // Commands whose usage text starts with `Usage:`, given a word too many,
    // and the others, short of a word.
    for cmd in [
        "json get k .a extra",
        "explain foo bar",
        "config list extra",
        "audit last 1 extra",
        "timezone UTC extra",
        "format iso extra",
        "workspace list x y",
        "bump k add 5 extra",
        "convert k to json extra",
        "modify user",
        "set",
        "get where k:*.a = 1 limit 1 junk",
        "count",
        "grant",
        "range a",
        "create index",
    ] {
        let out = run(&mut s, cmd);
        assert!(!out.contains("Usage: Usage"), "{cmd}: {out}");
        assert!(out.matches("Usage:").count() <= 1, "{cmd}: {out}");
    }
    assert!(fails(&mut s, "json get k .a extra").contains("Usage: json get"));
}
