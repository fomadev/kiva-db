// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Numeric types of keys (lot N2 of docs/architecture/NUMERIC_TYPES.md,
//! section 10): `int` and `float` keys are numbers of code 2 told apart by
//! their text, a `decimal` key has code 13 and raises its file to format 3,
//! and numbers written before 2.0.0 read as they were.

mod common;

use common::{fails, ok, TempDir};
use kivadb::engine::{Engine, EngineOptions};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::{Type, Value};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::sync::Arc;

fn open(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

fn format_of(file: &Path) -> u32 {
    let h = std::fs::read(file).unwrap();
    u32::from_le_bytes([h[4], h[5], h[6], h[7]])
}

#[test]
fn keys_have_the_kind_they_were_written_with() {
    let dir = TempDir::new("nk_kinds");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "set n 42");
    ok(&mut s, "set f 1250.50");
    ok(&mut s, "set float g 8");
    ok(&mut s, "set int h -7");
    ok(&mut s, r#"set decimal d "1250.50""#);
    ok(&mut s, "set decimal e 0.10");
    ok(&mut s, "set number m 42");
    ok(&mut s, "set bool b true");
    assert_eq!(
        ok(&mut s, "typeof n and f and g and h and d and e and m and b"),
        " -> n is a [int]\n -> f is a [float]\n -> g is a [float]\n -> h is a [int]\n -> d is a [decimal]\n \
         -> e is a [decimal]\n -> m is a [int]\n -> b is a [boolean]"
    );
    assert_eq!(ok(&mut s, "get n"), "n: 42 [int]");
    assert_eq!(ok(&mut s, "get f"), "f: 1250.50 [float]", "the text as written");
    assert_eq!(ok(&mut s, "get g"), "g: 8.0 [float]");
    assert_eq!(ok(&mut s, "get d"), "d: 1250.50 [decimal]");
    assert_eq!(ok(&mut s, "get e"), "e: 0.10 [decimal]");

    // Typed reads, has, update.
    assert_eq!(ok(&mut s, "get int n"), "n: 42 [int]");
    assert!(fails(&mut s, "get float n").contains("is a [int]"));
    assert!(ok(&mut s, "has int n").starts_with("Yes"));
    assert!(ok(&mut s, "has float n").starts_with("No"));
    assert!(ok(&mut s, "has decimal d").starts_with("Yes"));
    ok(&mut s, "update n 43");
    ok(&mut s, r#"update decimal d "1300.00""#);
    assert!(fails(&mut s, "update d 5").contains("[decimal]"), "a decimal is not updated by a number");
    assert_eq!(ok(&mut s, "get d"), "d: 1300.00 [decimal]");
}

#[test]
fn what_no_type_holds_exactly_is_refused() {
    let dir = TempDir::new("nk_refusals");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    let e = fails(&mut s, "set big 12345678901234567890123");
    assert!(e.contains(r#"set decimal <key> "12345678901234567890123""#), "{e}");
    assert!(fails(&mut s, "set big 9223372036854775808").contains("out of range for int"));
    ok(&mut s, "set edge -9223372036854775808");
    ok(&mut s, "set edge2 9223372036854775807");
    let e = fails(&mut s, "set x 0.1000000000000000000001");
    assert!(e.contains("more digits than a float holds"), "{e}");
    ok(&mut s, r#"set decimal big "12345678901234567890123""#);
    assert_eq!(ok(&mut s, "get big"), "big: 12345678901234567890123 [decimal]");
    for bad in [
        "set int x 1.5",
        r#"set int x "42""#,
        "set float x 1e400",
        r#"set decimal x "1e3""#,
        r#"set decimal x "0.12345678901234567890123456789""#,
        "set x .5",
        "set x +1",
    ] {
        fails(&mut s, bad);
    }
    ok(&mut s, r#"set decimal z "-0.00""#);
    assert_eq!(ok(&mut s, "get z"), "z: 0.00 [decimal]");
    ok(&mut s, "set float nz -0");
    assert_eq!(ok(&mut s, "get nz"), "nz: 0.0 [float]");
}

#[test]
fn only_a_decimal_raises_a_file_to_format_3() {
    let dir = TempDir::new("nk_format");
    let path = dir.path().join("store.kiva");
    {
        let e = Engine::open(&path, EngineOptions::default()).unwrap();
        e.put("n", &Value::Num("42".into()), 0, false).unwrap();
        e.put("f", &Value::Num("8.0".into()), 0, false).unwrap();
        e.put("doc", &Value::from_json(json!({"a": 1})), 0, false).unwrap();
        drop(e);
    }
    assert!(format_of(&path) < 3, "int and float keys keep the format");
    {
        let e = Engine::open(&path, EngineOptions::default()).unwrap();
        e.compact().unwrap();
        drop(e);
    }
    assert!(format_of(&path) < 3, "compaction raises nothing");
    {
        let e = Engine::open(&path, EngineOptions::default()).unwrap();
        e.put("d", &Value::Decimal("1250.50".parse().unwrap()), 0, false).unwrap();
        drop(e);
    }
    assert_eq!(format_of(&path), 3);
    let e = Engine::open(&path, EngineOptions::default()).unwrap();
    let (v, entry) = e.get("d").unwrap().unwrap();
    assert_eq!(entry.ty, Type::Decimal);
    assert_eq!(v, Value::Decimal("1250.50".parse().unwrap()));
    assert_eq!(v.display(), "1250.50", "the scale is kept");
    e.compact().unwrap();
    assert_eq!(e.get("d").unwrap().unwrap().0.display(), "1250.50");
}

#[test]
fn numbers_written_before_2_0_0_read_as_they_were() {
    let dir = TempDir::new("nk_legacy");
    let inst = open(&dir);
    let e = inst.engine("default").unwrap();
    // Code-2 records as 1.x and the candidates wrote them, texts kept.
    for (k, text) in [
        ("a", "007"),
        ("b", "1250.50"),
        ("c", "12345678901234567890123"),
        ("d", "1e300"),
        ("e", "0.1000000000000000000001"),
        ("f", "2.5E-3"),
    ] {
        e.put(k, &Value::Num(text.into()), 0, false).unwrap();
    }
    drop(e);
    let mut s = Session::new(inst.clone(), true, None);
    assert_eq!(
        ok(&mut s, "typeof a and b and c and d and e and f"),
        " -> a is a [int]\n -> b is a [float]\n -> c is a [number]\n -> d is a [float]\n -> e is a [number]\n \
         -> f is a [float]"
    );
    assert_eq!(ok(&mut s, "get a"), "a: 007 [int]", "shown as written");
    assert_eq!(ok(&mut s, "get c"), "c: 12345678901234567890123 [number]");
    // Arithmetic on a legacy number is refused, never rounded.
    assert!(fails(&mut s, "print c + 1").contains("cannot be held exactly"));
    // Programs get what they got before.
    let get = |s: &mut Session, k: &str| kivadb::api::handle(s, &json!({"op": "get", "key": k})).unwrap();
    assert_eq!(get(&mut s, "b")["value"], json!(1250.5));
    assert_eq!(get(&mut s, "b")["type"], "number");
    assert_eq!(get(&mut s, "c")["value"], json!("12345678901234567890123"));
}

#[test]
fn rest_takes_and_gives_the_three_kinds() {
    let dir = TempDir::new("nk_rest");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    let call = |s: &mut Session, j: serde_json::Value| kivadb::api::handle(s, &j);
    call(&mut s, json!({"op": "put", "key": "i", "type": "int", "value": 42})).unwrap();
    call(&mut s, json!({"op": "put", "key": "f", "type": "float", "value": 8})).unwrap();
    call(&mut s, json!({"op": "put", "key": "d", "type": "decimal", "value": "1250.50"})).unwrap();
    assert!(call(&mut s, json!({"op": "put", "key": "x", "type": "decimal", "value": 1250.5})).is_err());
    assert!(call(&mut s, json!({"op": "put", "key": "x", "type": "int", "value": 1.5})).is_err());
    assert_eq!(ok(&mut s, "typeof i and f and d"), " -> i is a [int]\n -> f is a [float]\n -> d is a [decimal]");
    let d = call(&mut s, json!({"op": "get", "key": "d"})).unwrap();
    assert_eq!(d["value"], json!("1250.50"), "plain JSON carries a decimal as its text");
    assert_eq!(d["type"], "decimal");
    assert_eq!(call(&mut s, json!({"op": "get", "key": "i"})).unwrap()["value"], json!(42));
    assert_eq!(call(&mut s, json!({"op": "get", "key": "f"})).unwrap()["value"], json!(8.0));
}

/// One RESP command, its reply as text.
fn resp(stream: &mut BufReader<std::net::TcpStream>, args: &[&str]) -> String {
    let mut req = format!("*{}\r\n", args.len());
    for a in args {
        req.push_str(&format!("${}\r\n{a}\r\n", a.len()));
    }
    stream.get_mut().write_all(req.as_bytes()).unwrap();
    let mut line = String::new();
    stream.read_line(&mut line).unwrap();
    let line = line.trim_end().to_string();
    match line.as_bytes().first() {
        Some(b'$') => {
            let n: i64 = line[1..].parse().unwrap();
            if n < 0 {
                return "(nil)".into();
            }
            let mut body = vec![0u8; n as usize + 2];
            stream.read_exact(&mut body).unwrap();
            String::from_utf8_lossy(&body[..n as usize]).into_owned()
        }
        _ => line,
    }
}

#[test]
fn redis_and_lua_see_each_kind() {
    let dir = TempDir::new("nk_redis");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "set n 41");
    ok(&mut s, r#"set decimal d "1250.50""#);
    ok(&mut s, r#"set decimal five "5""#);
    let port = kivadb::resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut c = BufReader::new(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap());
    assert_eq!(resp(&mut c, &["GET", "d"]), "1250.50");
    assert_eq!(resp(&mut c, &["INCR", "n"]), ":42");
    assert_eq!(resp(&mut c, &["INCRBYFLOAT", "d", "0.25"]), "1250.75", "exact");
    assert_eq!(resp(&mut c, &["INCRBYFLOAT", "d", "0.1"]), "1250.85", "never through a float");
    assert!(resp(&mut c, &["INCR", "five"]).contains("decimal"), "a decimal is not made an int");
    assert!(resp(&mut c, &["INCRBYFLOAT", "d", "1e3"]).contains("not a valid decimal"));
    assert_eq!(ok(&mut s, "get d"), "d: 1250.85 [decimal]");
    assert_eq!(ok(&mut s, "get five"), "five: 5 [decimal]");
    assert_eq!(ok(&mut s, "get n"), "n: 42 [int]");
    // Lua: a decimal as its text, typeof by kind.
    assert_eq!(
        ok(&mut s, r#"eval 'return kiva.get("d"), kiva.typeof("d"), kiva.typeof("n")'"#),
        "1250.85, decimal, int"
    );
}

#[test]
fn json_type_names_int_and_float_in_documents() {
    let dir = TempDir::new("nk_json_type");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, r#"set doc {"a": 1, "b": 1.5, "c": 8.0, "d": 18446744073709551615, "s": "x"}"#);
    assert_eq!(ok(&mut s, "json type doc .a"), "doc.a: [int]");
    assert_eq!(ok(&mut s, "json type doc .b"), "doc.b: [float]");
    assert_eq!(ok(&mut s, "json get doc .c"), "doc.c: 8.0 [float]");
    assert_eq!(ok(&mut s, "json type doc .d"), "doc.d: [decimal]", "a u64 of 2.0 reads as a decimal (H-u64)");
    assert_eq!(ok(&mut s, "json get doc .s"), "doc.s: x [string]");
    assert_eq!(ok(&mut s, "get doc.a"), "doc.a: 1 [int]");
    ok(&mut s, r#"set t {"p": decimal "9.90", "q": int 3}"#);
    assert_eq!(ok(&mut s, "json type t .q"), "t.q: [int]");
    assert_eq!(ok(&mut s, "get t.p"), "t.p: 9.90 [decimal]");
}

#[test]
fn a_decimal_key_travels_through_transactions_and_replication() {
    let dir = TempDir::new("nk_txn");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "begin");
    ok(&mut s, r#"set decimal d "10.00""#);
    ok(&mut s, "set n 1");
    assert_eq!(ok(&mut s, "typeof d"), " -> d is a [decimal]");
    ok(&mut s, "commit");
    assert_eq!(ok(&mut s, "get d"), "d: 10.00 [decimal]");
    ok(&mut s, "begin");
    ok(&mut s, r#"update decimal d "11.00""#);
    ok(&mut s, "rollback");
    assert_eq!(ok(&mut s, "get d"), "d: 10.00 [decimal]");

    // The bytes a replica receives give the same decimal.
    let e = inst.engine("default").unwrap();
    let (v, _) = e.get("d").unwrap().unwrap();
    let raw = v.encode(false).unwrap();
    let replica_dir = TempDir::new("nk_replica");
    let replica = Engine::open(replica_dir.path().join("r.kiva"), EngineOptions::default()).unwrap();
    replica.put_raw("d", Type::Decimal, &raw, kivadb::value::DocEnc::Json, 0, 0).unwrap();
    assert_eq!(replica.get("d").unwrap().unwrap().0, Value::Decimal("10.00".parse().unwrap()));
}
