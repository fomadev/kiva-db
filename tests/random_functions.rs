// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Random values and UUIDs, lot R1 of docs/architecture/RANDOM_FUNCTIONS.md
//! (section 12): `random()`, `rand()`, `random_int`, `random_string`,
//! `random_bytes`, `uuid()` and `uuid_v7()` in commands, documents,
//! `json set`, `print`, schema defaults and Lua. Each is drawn once, and only
//! the value is stored: transactions, the change journal, replicas and
//! backups carry the value, never the call.

mod common;

use common::{fails, instance, ok, TempDir};
use kivadb::instance::Instance;
use kivadb::server;
use kivadb::session::Session;
use kivadb::value::Value;
use serde_json::Value as Json;
use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn value(inst: &Instance, key: &str) -> Value {
    inst.engine("default").unwrap().get(key).unwrap().unwrap_or_else(|| panic!("{key} is missing")).0
}

fn text(inst: &Instance, key: &str) -> String {
    match value(inst, key) {
        Value::Str(s) => s,
        other => panic!("{key} is not a text: {other:?}"),
    }
}

fn doc(inst: &Instance, key: &str) -> Json {
    match value(inst, key) {
        Value::Doc(j) => j,
        other => panic!("{key} is not a JSON document: {other:?}"),
    }
}

fn is_uuid(s: &str, version: char) -> bool {
    let b = s.as_bytes();
    s.len() == 36
        && [8, 13, 18, 23].iter().all(|&i| b[i] == b'-')
        && s.chars().enumerate().all(|(i, c)| [8, 13, 18, 23].contains(&i) || matches!(c, '0'..='9' | 'a'..='f'))
        && b[14] == version as u8
        && matches!(b[19], b'8' | b'9' | b'a' | b'b')
}

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    while !cond() {
        assert!(Instant::now() < until, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn every_function_gives_a_value_of_its_type() {
    let dir = TempDir::new("random_values");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    for cmd in [
        "set id uuid()",
        "set id7 uuid_v7()",
        "set n random_int(-3, 3)",
        "set f random()",
        "set f2 RAND()",
        "set s random_string(16)",
        "set b random_bytes(32)",
        "set int roll random_int(1, 6)",
        "set float g random()",
        "set big random_int(-9223372036854775808, 9223372036854775807)",
    ] {
        ok(&mut s, cmd);
    }
    assert!(is_uuid(&text(&inst, "id"), '4'), "{}", text(&inst, "id"));
    assert!(is_uuid(&text(&inst, "id7"), '7'), "{}", text(&inst, "id7"));
    let s16 = text(&inst, "s");
    assert!(s16.len() == 16 && s16.bytes().all(|c| c.is_ascii_alphanumeric()), "{s16}");
    let b = text(&inst, "b");
    assert!(b.len() == 43 && b.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'), "{b}");
    let num = |k: &str| match value(&inst, k) {
        Value::Num(n) => n,
        other => panic!("{k} is not a number: {other:?}"),
    };
    assert!((-3..=3).contains(&num("n").parse::<i64>().unwrap()));
    assert!((1..=6).contains(&num("roll").parse::<i64>().unwrap()));
    num("big").parse::<i64>().unwrap();
    for k in ["f", "f2", "g"] {
        let f = num(k);
        assert!(f.contains('.'), "a float shows its fraction: {f}");
        assert!((0.0..1.0).contains(&f.parse::<f64>().unwrap()));
    }
    assert!(ok(&mut s, "typeof n").contains("[int]"));
    assert!(ok(&mut s, "typeof f").contains("[float]"));
    assert!(ok(&mut s, "typeof id").contains("[string]"));

    // A forced type the function does not give is refused.
    assert!(fails(&mut s, "set string t random_int(1, 2)").contains("not a string"));
    // In quotes, a call is a text; `uuid "..."` stays a type word in a document.
    ok(&mut s, r#"set q "uuid()""#);
    assert_eq!(text(&inst, "q"), "uuid()");
    ok(&mut s, r#"set u {"id": uuid "550E8400-E29B-41D4-A716-446655440000"}"#);
    assert_eq!(doc(&inst, "u")["id"], "550e8400-e29b-41d4-a716-446655440000");
}

#[test]
fn each_call_draws_again() {
    let dir = TempDir::new("random_distinct");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let mut seen = HashSet::new();
    let mut previous_v7 = String::new();
    for i in 0..200 {
        ok(&mut s, &format!("set a:{i} uuid() and b:{i} uuid_v7() and c:{i} random_string(12)"));
        assert!(seen.insert(text(&inst, &format!("a:{i}"))));
        assert!(seen.insert(text(&inst, &format!("c:{i}"))));
        let v7 = text(&inst, &format!("b:{i}"));
        assert!(v7 > previous_v7, "uuid_v7 increases: {v7} after {previous_v7}");
        previous_v7 = v7;
    }
}

#[test]
fn functions_inside_documents_json_set_and_print() {
    let dir = TempDir::new("random_documents");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(
        &mut s,
        r#"set q:1 {"id": uuid_v7(), "score": random_int(0, 100), "weight": random(),
                    "tags": [random_string(4), random_bytes(3)], "at": now()}"#,
    );
    let out = ok(&mut s, "get q:1.id");
    let id = out.split(": ").nth(1).unwrap().split_whitespace().next().unwrap().trim_matches('"').to_string();
    assert!(is_uuid(&id, '7'), "{out}");
    let score: i64 =
        ok(&mut s, "get q:1.score").split(": ").nth(1).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert!((0..=100).contains(&score));
    // Without a moment or a type word, the document stays JSON: the functions gave JSON values.
    ok(&mut s, r#"set q:2 {"id": uuid(), "n": random_int(5, 5)}"#);
    let q2 = doc(&inst, "q:2");
    assert!(is_uuid(q2["id"].as_str().unwrap(), '4'));
    assert_eq!(q2["n"], 5);

    ok(&mut s, "json set q:2 .token random_string(8)");
    ok(&mut s, "json set q:2 .roll random_int(1, 1)");
    ok(&mut s, r#"set list {"items": ["a"]}"#);
    ok(&mut s, "json append list .items uuid(), random_int(1, 1)");
    ok(&mut s, "json insert list .items[0] random_string(3)");
    let q2 = doc(&inst, "q:2");
    assert_eq!(q2["token"].as_str().unwrap().len(), 8);
    assert_eq!(q2["roll"], 1);
    let items = doc(&inst, "list")["items"].clone();
    assert_eq!(items[0].as_str().unwrap().len(), 3);
    assert!(is_uuid(items[2].as_str().unwrap(), '4'));
    assert_eq!(items[3], 1);

    let printed = ok(&mut s, "print random_int(7, 7), uuid(), random_int(2 + 3, 5)");
    assert!(printed.starts_with("7 ") && printed.trim_end().ends_with(" 5"), "{printed}");

    // Errors name the call and leave nothing written.
    for (cmd, says) in [
        ("set e random_int(5, 1)", "lower bound is above"),
        ("set e random_string(0)", "from 1 to 1024"),
        ("set e random_bytes(1025)", "from 1 to 1024"),
        ("set e random_int(1.5, 2)", "not a whole number"),
        ("set e uuid(1)", "no argument"),
        ("set e random_int(1, 2", "')'"),
        (r#"set e {"a": random_string(0)}"#, "(.a): random_string"),
        ("json set q:2 .x random_int(2, 1)", "lower bound is above"),
        ("print random_string(2000)", "from 1 to 1024"),
    ] {
        assert!(fails(&mut s, cmd).contains(says), "{cmd}");
    }
    assert!(inst.engine("default").unwrap().get("e").unwrap().is_none());
    assert!(doc(&inst, "q:2").get("x").is_none());
}

#[test]
fn shuffle_and_sample_arrays() {
    let dir = TempDir::new("random_arrays");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let sorted = |v: &Json| {
        let mut items: Vec<String> = v.as_array().unwrap().iter().map(|x| x.to_string()).collect();
        items.sort();
        items
    };
    ok(&mut s, r#"set a shuffle(["A", "B", "C", "D", "E"])"#);
    assert_eq!(sorted(&doc(&inst, "a")), ["\"A\"", "\"B\"", "\"C\"", "\"D\"", "\"E\""]);
    ok(&mut s, "set one sample([10, 20, 30])");
    assert!(["10", "20", "30"].contains(&ok(&mut s, "get one").split(": ").nth(1).unwrap().split(' ').next().unwrap()));
    ok(&mut s, "set none sample([1, 2], 0) and empty shuffle([])");
    assert_eq!(doc(&inst, "none"), serde_json::json!([]));
    assert_eq!(doc(&inst, "empty"), serde_json::json!([]));
    // Elements keep their types, typed values included.
    ok(&mut s, r#"set typed shuffle([decimal "1.50", date "2026-01-15", int 3, float 0.5])"#);
    let shown = ok(&mut s, "get typed");
    for t in [r#"decimal "1.50""#, r#"date "2026-01-15""#, "3", "0.5"] {
        assert!(shown.contains(t), "{t} in {shown}");
    }
    ok(&mut s, r#"set d sample([date "2026-01-15"])"#);
    assert!(ok(&mut s, "typeof d").contains("[date]"));
    ok(&mut s, r#"set o sample([{"a": 1}])"#);
    assert_eq!(doc(&inst, "o"), serde_json::json!({"a": 1}));

    // Inside documents, nested with other functions, and in json set.
    ok(
        &mut s,
        r#"set quiz:1 {"questions": shuffle(["Q1", "Q2", "Q3", "Q4"]), "bonus": sample(["X", "Y"]),
                       "three": sample([1, 2, 3, 4, 5], 3), "ids": shuffle([uuid(), uuid()])}"#,
    );
    let q = doc(&inst, "quiz:1");
    assert_eq!(sorted(&q["questions"]), ["\"Q1\"", "\"Q2\"", "\"Q3\"", "\"Q4\""]);
    assert!(q["bonus"] == "X" || q["bonus"] == "Y");
    let three = q["three"].as_array().unwrap();
    assert_eq!(three.len(), 3);
    assert_eq!(three.iter().collect::<HashSet<_>>().len(), 3);
    assert!(is_uuid(q["ids"][0].as_str().unwrap(), '4'));
    ok(&mut s, "json set quiz:1 .order shuffle([1, 2, 3])");
    assert_eq!(sorted(&doc(&inst, "quiz:1")["order"]), ["1", "2", "3"]);
    ok(&mut s, r#"json append quiz:1 .questions sample(["Q5", "Q6"])"#);
    assert_eq!(doc(&inst, "quiz:1")["questions"].as_array().unwrap().len(), 5);

    // Over many draws, every order of three comes up.
    let mut orders = HashSet::new();
    for i in 0..200 {
        ok(&mut s, &format!("set p:{i} shuffle([1, 2, 3])"));
        orders.insert(doc(&inst, &format!("p:{i}")).to_string());
    }
    assert_eq!(orders.len(), 6, "{orders:?}");

    for (cmd, says) in [
        ("set x sample([])", "empty array has no element"),
        ("set x sample([1, 2], 3)", "3 elements asked of an array of 2"),
        ("set x sample([1, 2], -1)", "0 or more"),
        (r#"set x shuffle("abc")"#, "takes an array"),
        ("set x shuffle([1], 2)", "takes one array"),
        (r#"set x {"a": shuffle(3)}"#, "shuffle takes an array, not int"),
        (r#"set x {"a": sample([1], 5)}"#, "5 elements asked of an array of 1"),
        ("json set quiz:1 .x sample([])", "empty array"),
        ("print shuffle([1, 2])", "print does not hold"),
        ("get where quiz:*.bonus = sample([\"X\"])", "no place in a condition"),
    ] {
        assert!(fails(&mut s, cmd).contains(says), "{cmd}");
    }
    assert!(inst.engine("default").unwrap().get("x").unwrap().is_none());

    // Lua: a new list, the one given left as it was.
    let out = ok(
        &mut s,
        r#"eval 'local t = {1, 2, 3, 4} local u = kiva.shuffle(t) table.sort(u) return t[1] .. t[2] .. t[3] .. t[4], u[1] .. u[4], #kiva.sample(t, 2), kiva.sample({9})'"#,
    );
    assert_eq!(out.trim(), "1234, 14, 2, 9");
    ok(&mut s, r#"eval 'kiva.set("lq", kiva.shuffle({"a", "b", "c"})) return 1'"#);
    assert_eq!(sorted(&doc(&inst, "lq")), ["\"a\"", "\"b\"", "\"c\""]);
    assert!(fails(&mut s, "eval 'return kiva.sample({}, 1)'").contains("1 element asked of an array of 0"));
    assert!(fails(&mut s, "eval 'return kiva.shuffle(3)'").contains("takes a list"));
}

/// The keys of the rows a `get where` printed.
fn row_keys(out: &str) -> Vec<String> {
    out.lines().filter(|l| !l.starts_with('(')).filter_map(|l| l.split(": ").next()).map(String::from).collect()
}

#[test]
fn order_by_random_draws_among_the_documents_selected() {
    let dir = TempDir::new("random_order");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    for i in 0..40 {
        let role = if i % 4 == 0 { "admin" } else { "user" };
        ok(&mut s, &format!(r#"set u:{i:02} {{"role": "{role}", "n": {i}}}"#));
    }
    // Not documents of the pattern: never drawn.
    ok(&mut s, r#"set u:text "a text" and v:1 {"role": "user"}"#);

    // A pattern alone: drawn among its documents.
    let rows = row_keys(&ok(&mut s, "get where u:* order by random() limit 5"));
    assert_eq!(rows.len(), 5);
    assert_eq!(rows.iter().collect::<HashSet<_>>().len(), 5, "{rows:?}");
    assert!(rows.iter().all(|k| k.starts_with("u:") && k != "u:text"), "{rows:?}");
    // More asked than there are: all of them, once each.
    let all = row_keys(&ok(&mut s, "get where u:* order by random() limit 100"));
    assert_eq!(all.len(), 40);
    assert_eq!(all.iter().collect::<HashSet<_>>().len(), 40);

    // With a condition: only rows that pass, `return` applied.
    for _ in 0..20 {
        let out = ok(&mut s, r#"get where u:*.role = "admin" order by random() limit 3 return n"#);
        for k in row_keys(&out) {
            let n: usize = k[2..].parse().unwrap();
            assert_eq!(n % 4, 0, "{out}");
        }
        assert!(out.contains("(3 matches)") && !out.contains("role"), "{out}");
    }
    assert!(ok(&mut s, r#"get where u:*.role = "admin" order by random() limit 50"#).contains("(10 matches)"));
    // An index changes how candidates are found, not what is drawn.
    ok(&mut s, "create index on u:* (role)");
    let out = ok(&mut s, r#"get where u:*.role = "admin" and u:*.n >= 20 order by random() limit 50"#);
    assert!(out.contains("(5 matches)"), "{out}");

    // Every document as likely: 1 of the 10 admins, 3 000 times (9 degrees, 27.9 at p = 0.001).
    let mut count = std::collections::HashMap::new();
    for _ in 0..3_000 {
        let k = row_keys(&ok(&mut s, r#"get where u:*.role = "admin" order by random() limit 1"#)).remove(0);
        *count.entry(k).or_insert(0f64) += 1.0;
    }
    assert_eq!(count.len(), 10);
    let chi: f64 = count.values().map(|c| (c - 300.0) * (c - 300.0) / 300.0).sum();
    assert!(chi < 27.9, "{count:?}");
    // And in a random order: the first of all 40 is not always the same.
    let firsts: HashSet<String> =
        (0..30).map(|_| row_keys(&ok(&mut s, "get where u:* order by random() limit 40")).remove(0)).collect();
    assert!(firsts.len() > 5, "{firsts:?}");

    for (cmd, says) in [
        ("get where u:* order by random()", "needs a limit from 1 to 10000"),
        ("get where u:*.n > 1 order by random() limit 10001", "needs a limit from 1 to 10000"),
        ("get where u:* order by random() limit 2 after u:01", "no next page"),
        ("get where u:* order by random() desc limit 2", "no direction"),
        ("get where u:* order by random() limit 2 reverse", "'reverse' has no meaning"),
        ("get where u:* order by random(3) limit 2", "nothing between its parentheses"),
        ("get where u:* order by random() order by n limit 2", "one 'order by' per query"),
        ("get where u:* order by n limit 2", "a pattern alone is read only with order by random()"),
        ("get where u:* limit 2", "expected a condition"),
    ] {
        assert!(fails(&mut s, cmd).contains(says), "{cmd}");
    }
}

#[test]
fn order_by_random_reads_only_the_documents_drawn_when_it_can() {
    let dir = TempDir::new("random_order_plan");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    for i in 0..200 {
        ok(&mut s, &format!(r#"set p:{i:03} {{"price": {i}}}"#));
    }
    // A pattern alone: every key looked at, 3 documents read.
    let out = ok(&mut s, "explain analyze get where p:* order by random() limit 3");
    assert!(out.contains("Order     : random: 3 drawn among the keys, then only those documents are read"), "{out}");
    assert!(out.contains("Keys examined : 200"), "{out}");
    assert!(out.contains("Documents     : 3 read"), "{out}");
    assert!(out.contains("Returned      : 3"), "{out}");
    // A condition: each candidate read and tested.
    let out = ok(&mut s, "explain analyze get where p:*.price < 50 order by random() limit 3");
    assert!(out.contains("random: 3 drawn among the documents that pass, every candidate read"), "{out}");
    assert!(out.contains("Documents     : 200 read") && out.contains("150 filtered out"), "{out}");
    ok(&mut s, "create index on p:* (price)");
    let out = ok(&mut s, "explain analyze get where p:*.price < 50 order by random() limit 3");
    assert!(out.contains("Documents     : 50 read") && out.contains("Returned      : 3"), "{out}");
    // explain alone runs nothing.
    let out = ok(&mut s, "explain get where p:* order by random() limit 3");
    assert!(out.contains("Limit     : 3") && !out.contains("Measured"), "{out}");
}

#[test]
fn order_by_random_skips_expired_keys_and_other_workspaces_and_changes_nothing() {
    let dir = TempDir::new("random_order_scope");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set k:1 {"a": 1} and k:2 {"a": 2}"#);
    ok(&mut s, r#"set k:3 {"a": 3} ttl 1"#);
    ok(&mut s, "workspace create other");
    ok(&mut s, "workspace switch other");
    ok(&mut s, r#"set k:9 {"a": 9}"#);
    ok(&mut s, "workspace switch default");
    std::thread::sleep(Duration::from_millis(1200));
    let before = ok(&mut s, "get where k:*.a > 0 limit 10");
    for _ in 0..20 {
        let mut rows = row_keys(&ok(&mut s, "get where k:* order by random() limit 10"));
        rows.sort();
        assert_eq!(rows, ["k:1", "k:2"]);
    }
    assert_eq!(ok(&mut s, "get where k:*.a > 0 limit 10"), before, "a read changes nothing");
}

/// A structured request posted to `/v1/api`, and its JSON answer.
fn rest(port: u16, body: &Json) -> (u16, Json) {
    use std::io::{Read, Write};
    let body = body.to_string();
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        s,
        "POST /v1/api HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    let status = answer.split_whitespace().nth(1).unwrap().parse().unwrap();
    let json = answer.split_once("\r\n\r\n").map(|(_, b)| serde_json::from_str(b).unwrap_or(Json::Null)).unwrap();
    (status, json)
}

#[test]
fn structured_requests_and_rest_draw_at_random() {
    use serde_json::json;
    let dir = TempDir::new("random_api");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    for i in 0..30 {
        ok(&mut s, &format!(r#"set a:{i:02} {{"n": {i}, "odd": {}}}"#, i % 2 == 1));
    }
    ok(&mut s, r#"set a:text "x""#);
    let keys = |answer: &Json| -> Vec<String> {
        answer["rows"].as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap().to_string()).collect()
    };

    // A pattern alone, and a condition.
    let r = kivadb::api::handle(&mut s, &json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 4}))
        .unwrap();
    let drawn = keys(&r);
    assert_eq!(drawn.len(), 4);
    assert_eq!(drawn.iter().collect::<HashSet<_>>().len(), 4);
    assert!(drawn.iter().all(|k| k != "a:text"));
    assert!(r["plan"]["order"].as_str().unwrap().contains("drawn among the keys"), "{r}");
    let r = kivadb::api::handle(
        &mut s,
        &json!({"op": "query", "target": "a:*", "where": {"odd": true}, "order_by": "RANDOM()", "limit": 100}),
    )
    .unwrap();
    let mut odd = keys(&r);
    assert_eq!(odd.len(), 15);
    odd.sort();
    assert_eq!(odd, (1..30).step_by(2).map(|i| format!("a:{i:02}")).collect::<Vec<_>>());
    // explain, and explain analyze: the documents drawn only.
    let r = kivadb::api::handle(
        &mut s,
        &json!({"op": "explain", "target": "a:*", "order_by": "random()", "limit": 3, "analyze": true}),
    )
    .unwrap();
    assert_eq!(r["analyze"]["documents_read"], 3, "{r}");
    assert_eq!(r["analyze"]["returned"], 3, "{r}");

    for (req, says) in [
        (json!({"op": "query", "target": "a:*", "order_by": "random()"}), "needs a \"limit\" from 1 to 10000"),
        (json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 10001}), "from 1 to 10000"),
        (json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 0}), "from 1 to 10000"),
        (json!({"op": "query", "target": "a:*", "order_by": "random() desc", "limit": 2}), "no direction"),
        (json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 2, "after": "a:01"}), "no next page"),
        (json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 2, "reverse": true}), "\"reverse\""),
        (json!({"op": "query", "target": "a:*.n", "order_by": "random()", "limit": 2}), "the key pattern alone"),
        (json!({"op": "query", "target": "a:*", "order_by": "n", "limit": 2}), "expected"),
        (json!({"op": "aggregate", "target": "a:*", "order_by": "random()"}), "sorting pipelines"),
    ] {
        let e = kivadb::api::handle(&mut s, &req).unwrap_err().to_string();
        assert!(e.contains(says), "{req}: {e}");
    }

    // The same through REST.
    let port = kivadb::http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let (status, r) = rest(port, &json!({"op": "query", "target": "a:*", "order_by": "random()", "limit": 2}));
    assert_eq!(status, 200, "{r}");
    assert_eq!(keys(&r).len(), 2);
    let (status, r) = rest(port, &json!({"op": "query", "target": "a:*", "order_by": "random()"}));
    assert_eq!(status, 400, "{r}");
}

#[test]
fn a_condition_refuses_a_random_function() {
    let dir = TempDir::new("random_conditions");
    let inst = instance(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, r#"set q:1 {"score": 3}"#);
    for cmd in [
        "get where q:*.score = random_int(1, 6)",
        "get where q:*.score in [random_int(1, 6), 2]",
        "count where q:*.score > random()",
    ] {
        assert!(fails(&mut s, cmd).contains("no place in a condition"), "{cmd}");
    }
}

#[test]
fn schema_defaults_draw_for_each_document() {
    let dir = TempDir::new("random_schema");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(
        &mut s,
        r#"schema set t:* {"id": {"type": "string", "default": "uuid_v7()", "unique": true},
                            "code": {"default": "random_string(6)"},
                            "roll": {"type": "integer", "default": "random_int(1, 6)"},
                            "w": {"type": "float", "default": "random()"}}"#,
    );
    ok(&mut s, r#"set t:1 {"name": "a"}"#);
    ok(&mut s, "begin");
    ok(&mut s, r#"set t:2 {"name": "b"}"#);
    ok(&mut s, r#"set t:3 {"name": "c", "id": "mine"}"#);
    ok(&mut s, "commit");
    let (a, b, c) = (doc(&inst, "t:1"), doc(&inst, "t:2"), doc(&inst, "t:3"));
    assert!(is_uuid(a["id"].as_str().unwrap(), '7') && is_uuid(b["id"].as_str().unwrap(), '7'));
    assert_ne!(a["id"], b["id"]);
    assert_eq!(c["id"], "mine", "a value given is kept");
    assert_eq!(a["code"].as_str().unwrap().len(), 6);
    assert!((1..=6).contains(&a["roll"].as_i64().unwrap()));
    assert!((0.0..1.0).contains(&a["w"].as_f64().unwrap()));

    for (spec, says) in [
        (r#"{"n": {"type": "string", "default": "random_int(1, 6)"}}"#, "does not give a value of type string"),
        (r#"{"n": {"type": "integer", "default": "uuid()"}}"#, "does not give a value of type"),
        (r#"{"n": {"type": "string", "default": "random_string(0)"}}"#, "in the default, random_string"),
    ] {
        assert!(fails(&mut s, &format!("schema set z:* {spec}")).contains(says), "{spec}");
    }
}

#[test]
fn lua_draws_from_the_same_functions() {
    let dir = TempDir::new("random_lua");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let out = ok(
        &mut s,
        r#"eval 'return kiva.random_int(4, 4), #kiva.random_string(9), #kiva.random_bytes(3), string.sub(kiva.uuid(), 15, 15), string.sub(kiva.uuid_v7(), 15, 15)'"#,
    );
    assert_eq!(out.trim(), "4, 9, 4, 4, 7");
    let f: f64 = ok(&mut s, "eval 'return kiva.random()'").trim().parse().unwrap();
    assert!((0.0..1.0).contains(&f));
    // A script writes the value it drew.
    ok(&mut s, r#"eval 'kiva.set("k", kiva.uuid_v7()) return 1'"#);
    assert!(is_uuid(&text(&inst, "k"), '7'));
    assert!(fails(&mut s, "eval 'return kiva.random_int(6, 1)'").contains("lower bound is above"));
    assert!(fails(&mut s, "eval 'return kiva.random_string(0)'").contains("from 1 to 1024"));
}

#[test]
fn a_rolled_back_transaction_leaves_nothing() {
    let dir = TempDir::new("random_rollback");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "begin");
    ok(&mut s, "set r uuid()");
    ok(&mut s, "rollback");
    assert!(inst.engine("default").unwrap().get("r").unwrap().is_none());
    ok(&mut s, "begin");
    ok(&mut s, "set r uuid() and r2 random_int(1, 9)");
    ok(&mut s, "commit");
    assert!(is_uuid(&text(&inst, "r"), '4'));
}

#[test]
fn a_replica_and_the_change_journal_hold_the_values_the_master_drew() {
    let (dir_m, dir_r) = (TempDir::new("random_master"), TempDir::new("random_replica"));
    let (master, replica) = (instance(&dir_m), instance(&dir_r));
    let port_m = server::start_tcp(&master, "127.0.0.1", 0).unwrap();
    server::start_tcp(&replica, "127.0.0.1", 0).unwrap();
    let mut m = Session::new(master.clone(), true, None);
    let mut r = Session::new(replica.clone(), true, None);
    ok(&mut m, "changes enable");
    ok(&mut m, r#"schema set p:* {"id": {"type": "string", "default": "uuid_v7()"}}"#);
    ok(&mut r, &format!("replica of 127.0.0.1 {port_m}"));
    wait_for("the initial snapshot", || replica.repl.link().status == "connected");
    ok(&mut m, "set id uuid() and roll random_int(1, 1000000)");
    ok(&mut m, r#"set p:1 {"n": random_int(1, 1000000), "f": random(), "order": shuffle([1, 2, 3, 4, 5, 6, 7, 8])}"#);
    ok(&mut m, "set pick sample([1, 2, 3, 4, 5, 6, 7, 8], 4)");
    ok(&mut m, r#"eval 'kiva.set("lua", kiva.random_string(20)) return 1'"#);
    ok(&mut m, "begin");
    ok(&mut m, "set t uuid_v7()");
    ok(&mut m, "commit");
    wait_for("the writes", || replica.engine("default").unwrap().contains("t"));
    for k in ["id", "roll", "p:1", "pick", "lua", "t"] {
        assert_eq!(value(&replica, k), value(&master, k), "{k}");
    }
    assert!(is_uuid(doc(&replica, "p:1")["id"].as_str().unwrap(), '7'));
    let journal = ok(&mut m, "changes read from beginning");
    for k in ["id", "lua", "t"] {
        assert!(journal.contains(&text(&master, k)), "{k} missing from the journal:\n{journal}");
    }
    assert!(!journal.contains("uuid()") && !journal.contains("random_"), "{journal}");
}

fn kivadb(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kivadb"))
        .arg("--home")
        .arg(home)
        .args(args)
        .env("KIVADB_NO_UPDATE_CHECK", "1")
        .env_remove("KIVADB_ENCRYPTION_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

#[test]
fn a_backup_restores_the_values_as_drawn() {
    let dir = TempDir::new("random_backup");
    let home = dir.path().join("home");
    let o = kivadb(
        &home,
        &["-e", "set id uuid_v7()", "-e", r#"set d {"code": random_string(10)}"#, "-e", "get id", "-e", "get d"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let before = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.starts_with("id:") || l.starts_with("d:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(before.lines().count(), 2, "{before}");
    let archive = dir.path().join("all.tar.gz");
    assert!(kivadb(&home, &["backup", "--output", archive.to_str().unwrap()]).status.success());
    let target = dir.path().join("restored");
    assert!(kivadb(&home, &["restore", archive.to_str().unwrap(), "--target", target.to_str().unwrap()])
        .status
        .success());
    let o = kivadb(&target, &["-e", "get id", "-e", "get d"]);
    let after = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.starts_with("id:") || l.starts_with("d:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(after, before);
}
