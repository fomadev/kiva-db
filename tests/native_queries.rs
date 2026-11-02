// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Documents holding decimals, dates, times and datetimes, read, queried,
//! indexed, aggregated, checked and changed through every path, their
//! values keeping their types (lot C3 of docs/architecture/NATIVE_DOCUMENTS.md).

mod common;

use common::{fails, ok, TempDir};
use kivadb::doc;
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::{DocEnc, Value};
use serde_json::{json, Value as Json};
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::Arc;

fn open(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

fn typed(j: Json) -> Value {
    Value::typed(doc::from_extended_json(&j).unwrap()).unwrap()
}

/// What a key holds, in extended JSON: every type told apart.
fn ext(inst: &Instance, key: &str) -> Json {
    match inst.engine("default").unwrap().get(key).unwrap().unwrap().0 {
        Value::Typed(d) => doc::to_extended_json(&d),
        Value::Doc(j) => j,
        other => panic!("not a document: {other:?}"),
    }
}

fn enc(inst: &Instance, key: &str) -> DocEnc {
    inst.engine("default").unwrap().get(key).unwrap().unwrap().1.doc_enc()
}

fn put(inst: &Instance, key: &str, j: Json) {
    inst.engine("default").unwrap().put(key, &typed(j), 0, false).unwrap();
}

/// A deeply nested document with every native type.
fn account() -> Json {
    json!({
        "name": "Fordi",
        "balance": {"$decimal": "1250.50"},
        "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
        "profile": {"born": {"$date": "2004-05-09"}, "opens": {"$time": "08:30:00"},
                    "levels": [[{"$decimal": "0.1"}, {"$decimal": "0.2"}], [1, 2.5, "x"]]},
        "tags": ["a", {"$date": "2027-01-01"}]
    })
}

#[test]
fn shell_changes_keep_the_types_of_the_values_they_do_not_touch() {
    let dir = TempDir::new("nq_shell");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    put(&inst, "acc:1", account());

    ok(&mut s, r#"json set acc:1 .name "Fordima""#);
    ok(&mut s, "json set acc:1 .profile.levels[1][0] 7");
    ok(&mut s, r#"json append acc:1 .tags "b""#);
    ok(&mut s, r#"json insert acc:1 .tags[0] "first""#);
    ok(&mut s, "json delete acc:1 .profile.levels[1][2]");
    assert_eq!(ok(&mut s, "json incr acc:1 .balance 10"), "acc:1.balance: 1260.50 [decimal]");
    assert_eq!(ok(&mut s, "json incr acc:1 .profile.levels[1][0] 1"), "acc:1.profile.levels[1][0]: 8 [number]");
    let err = fails(&mut s, "json incr acc:1 .balance 1e-1");
    assert!(err.contains("do not mix"), "{err}");
    assert_eq!(
        ext(&inst, "acc:1"),
        json!({
            "name": "Fordima",
            "balance": {"$decimal": "1260.50"},
            "created_at": {"$datetime": "2026-10-31T16:18:55.000Z"},
            "profile": {"born": {"$date": "2004-05-09"}, "opens": {"$time": "08:30:00"},
                        "levels": [[{"$decimal": "0.1"}, {"$decimal": "0.2"}], [8, 2.5]]},
            "tags": ["first", "a", {"$date": "2027-01-01"}, "b"]
        })
    );
    assert_eq!(enc(&inst, "acc:1"), DocEnc::Kdoc);

    // Reading tells the types.
    ok(&mut s, "timezone UTC");
    assert_eq!(ok(&mut s, "json get acc:1 .balance"), "acc:1.balance: 1260.50 [decimal]");
    assert_eq!(ok(&mut s, "json type acc:1 .profile.born"), "acc:1.profile.born: [date]");
    assert_eq!(ok(&mut s, "json get acc:1 .profile.born"), "acc:1.profile.born: 2004-05-09 [date]");
    assert_eq!(ok(&mut s, "json get acc:1 .profile.opens"), "acc:1.profile.opens: 08:30:00 [time]");
    assert_eq!(ok(&mut s, "json len acc:1 .tags"), "acc:1.tags: 4");
    assert_eq!(ok(&mut s, "get acc:1.profile.born"), "acc:1.profile.born: 2004-05-09 [date]");

    // A document that loses its last native value goes back to JSON.
    put(&inst, "small", json!({"d": {"$decimal": "1.5"}, "n": 1}));
    ok(&mut s, "json delete small .d");
    assert_eq!(enc(&inst, "small"), DocEnc::Json);
    assert_eq!(ext(&inst, "small"), json!({"n": 1}));
}

#[test]
fn transactions_keep_types_and_take_back_everything_on_rollback() {
    let dir = TempDir::new("nq_txn");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    put(&inst, "acc:1", account());
    put(&inst, "acc:2", json!({"balance": {"$decimal": "10.00"}}));

    ok(&mut s, "begin");
    ok(&mut s, "json incr acc:1 .balance -100.25");
    ok(&mut s, "json incr acc:2 .balance 100.25");
    ok(&mut s, "commit");
    assert_eq!(ext(&inst, "acc:1")["balance"], json!({"$decimal": "1150.25"}));
    assert_eq!(ext(&inst, "acc:2")["balance"], json!({"$decimal": "110.25"}));
    assert_eq!(ext(&inst, "acc:1")["created_at"], json!({"$datetime": "2026-10-31T16:18:55.000Z"}));

    ok(&mut s, "begin");
    ok(&mut s, "json incr acc:1 .balance 1");
    ok(&mut s, r#"json set acc:2 .note "x""#);
    ok(&mut s, "rollback");
    assert_eq!(ext(&inst, "acc:1")["balance"], json!({"$decimal": "1150.25"}));
    assert!(ext(&inst, "acc:2").get("note").is_none());
}

#[test]
fn queries_compare_decimals_by_their_exact_value_in_every_encoding() {
    let dir = TempDir::new("nq_queries");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let e = inst.engine("default").unwrap();
    // The same field: a decimal in KDOC, numbers in JSON and MessagePack.
    put(&inst, "p:1", json!({"price": {"$decimal": "10.50"}, "day": {"$date": "2026-10-31"}}));
    put(&inst, "p:2", json!({"price": {"$decimal": "0.1"}, "day": {"$date": "2026-11-01"}}));
    ok(&mut s, r#"set p:3 {"price": 10.5, "day": "2026-10-30"}"#);
    e.put("p:4", &Value::from_json(json!({"price": 3, "day": "2026-12-01"})), 0, true).unwrap();
    ok(&mut s, r#"set p:5 {"price": 0.1, "day": "2027-01-01"}"#);

    let both = |s: &mut Session| {
        assert_eq!(ok(s, "count where p:*.price = 10.5"), "Count: 2", "10.50 and 10.5 are one value");
        assert_eq!(ok(s, "count where p:*.price > 3"), "Count: 2");
        assert_eq!(ok(s, "count where p:*.price >= 3"), "Count: 3");
        assert_eq!(ok(s, "count where p:*.price < 1"), "Count: 2");
        // The decimal 0.1 is not the float 0.1 (0.1000000000000000055...).
        assert_eq!(ok(s, "count where p:*.price = 0.1"), "Count: 1");
        assert_eq!(ok(s, "count where p:*.price in [3, 10.5]"), "Count: 3");
        // Dates, native or written as ISO texts, compare as moments.
        assert_eq!(ok(s, r#"count where p:*.day >= "2026-11-01""#), "Count: 3");
        let order = ok(s, "get where p:*.price > 0 order by price desc");
        let keys: Vec<String> = order.lines().map(|l| l.split(':').take(2).collect::<Vec<_>>().join(":")).collect();
        assert_eq!(&keys[..3], ["p:1", "p:3", "p:4"], "{order}");
    };
    both(&mut s);
    ok(&mut s, "create index on p:*.price");
    assert!(ok(&mut s, "explain get where p:*.price = 10.5").contains("INDEX SCAN"));
    both(&mut s);
    drop((s, e));
    inst.shutdown();
    drop(inst);
    // The index is rebuilt from the three encodings at opening.
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    both(&mut s);
}

#[test]
fn unique_indexes_take_equal_numbers_for_one_value_whatever_their_types() {
    let dir = TempDir::new("nq_unique");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let e = inst.engine("default").unwrap();
    ok(&mut s, "create unique index on c:*.code");
    put(&inst, "c:1", json!({"code": {"$decimal": "10.50"}, "at": {"$time": "08:00:00"}}));
    // 10.5 the float, and 10.5 the decimal written another way: duplicates.
    assert!(fails(&mut s, r#"set c:2 {"code": 10.5}"#).contains("unique index"));
    assert!(e
        .put("c:3", &typed(json!({"code": {"$decimal": "10.5"}, "d": {"$date": "2026-01-01"}})), 0, false)
        .is_err());
    // The float 0.1 and the decimal 0.1 are two values.
    put(&inst, "c:4", json!({"code": {"$decimal": "0.1"}, "d": {"$date": "2026-01-01"}}));
    ok(&mut s, r#"set c:5 {"code": 0.1}"#);
    // An integer and a decimal with no fraction: one value.
    ok(&mut s, r#"set c:6 {"code": 7}"#);
    assert!(e
        .put("c:7", &typed(json!({"code": {"$decimal": "7.000"}, "d": {"$date": "2026-01-01"}})), 0, false)
        .is_err());
}

#[test]
fn aggregates_add_decimals_exactly_and_refuse_mixing_them_with_floats() {
    let dir = TempDir::new("nq_aggregates");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    put(&inst, "o:1", json!({"city": "Goma", "total": {"$decimal": "0.1"}, "at": {"$date": "2026-10-01"}}));
    put(&inst, "o:2", json!({"city": "Goma", "total": {"$decimal": "0.2"}, "at": {"$date": "2026-10-02"}}));
    put(&inst, "o:3", json!({"city": "Bukavu", "total": {"$decimal": "10.50"}, "at": {"$date": "2026-11-02"}}));
    ok(&mut s, r#"set o:4 {"city": "Bukavu", "total": 2}"#);

    // 0.1 + 0.2 is 0.3 exactly, not 0.30000000000000004.
    assert!(ok(&mut s, r#"sum o:*.total where o:*.city = "Goma""#).contains("0.3"), "the exact sum");
    let all = ok(&mut s, "sum o:*.total");
    assert!(all.contains("12.80"), "{all}");
    assert!(ok(&mut s, "avg o:*.total").contains("3.2"));
    assert!(ok(&mut s, "max o:*.total").contains("10.50"));
    assert!(ok(&mut s, "min o:*.total").contains("0.1"));
    let grouped = ok(&mut s, "sum o:*.total group by city");
    assert!(grouped.contains("Goma") && grouped.contains("0.3") && grouped.contains("12.50"), "{grouped}");
    // The month of a native date.
    assert!(ok(&mut s, "count o:* group by month(at)").contains("11"));

    ok(&mut s, r#"set o:5 {"city": "Uvira", "total": 0.5}"#);
    let err = fails(&mut s, "sum o:*.total");
    assert!(err.contains("do not mix"), "{err}");

    // Through the structured requests too.
    let agg = json!({"op": "aggregate", "target": "o:*", "where": {"field": "city", "cmp": "=", "value": "Goma"},
                     "compute": [{"sum": "total"}]});
    let got = kivadb::api::handle(&mut s, &agg).unwrap();
    assert_eq!(got["values"]["sum_total"], json!("0.3"), "{got}");
}

#[test]
fn schemas_know_decimals_dates_and_times_and_fill_defaults_into_typed_documents() {
    let dir = TempDir::new("nq_schemas");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    let e = inst.engine("default").unwrap();
    ok(
        &mut s,
        r#"schema set acc: {"balance": {"type": "decimal", "required": true, "min": 0, "max": 1000000},
                             "rate": {"type": "number"}, "born": {"type": "date"}, "count": {"type": "int"},
                             "status": {"type": "string", "default": "open"}}"#,
    );
    let write = |j: Json| e.put("acc:1", &typed(j), 0, false);
    write(json!({"balance": {"$decimal": "12.50"}, "rate": {"$decimal": "0.05"}, "born": {"$date": "2004-05-09"},
                 "count": 3}))
    .unwrap();
    // The default is added, the native values kept.
    assert_eq!(
        ext(&inst, "acc:1"),
        json!({"balance": {"$decimal": "12.50"}, "rate": {"$decimal": "0.05"}, "born": {"$date": "2004-05-09"},
               "count": 3, "status": "open"})
    );
    let err =
        write(json!({"balance": {"$decimal": "-0.01"}, "born": {"$date": "2004-05-09"}})).unwrap_err().to_string();
    assert!(err.contains("below the minimum"), "{err}");
    let err = write(json!({"balance": 12.5, "born": {"$date": "2004-05-09"}})).unwrap_err().to_string();
    assert!(err.contains("expected of type decimal"), "{err}");
    let err = write(json!({"balance": {"$decimal": "1"}, "count": 1.5, "born": {"$date": "2004-05-09"}}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("of type int"), "{err}");
    // A date written as an ISO text still follows a date rule.
    assert!(write(json!({"balance": {"$decimal": "1"}, "born": "2004-05-09", "at": {"$time": "08:00:00"}})).is_ok());
    assert!(ok(&mut s, "schema check acc:").contains("all follow the schema"));
}

#[test]
fn changes_and_replication_carry_the_types_of_a_partial_change() {
    let dir = TempDir::new("nq_changes");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "changes enable");
    put(&inst, "acc:1", account());
    ok(&mut s, r#"json set acc:1 .name "Fordima""#);
    let e = inst.engine("default").unwrap();
    let r = e.read_changes(kivadb::engine::ChangesFrom::Beginning, 10, None, None).unwrap();
    let last = r.changes.last().unwrap();
    assert_eq!(last.enc, DocEnc::Kdoc);
    let raw = last.value.clone().unwrap();
    // The same bytes, applied on a replica, give the same typed document.
    let replica_dir = TempDir::new("nq_changes_replica");
    let replica = kivadb::engine::Engine::open(replica_dir.path().join("r.kiva"), Default::default()).unwrap();
    replica.put_raw("acc:1", kivadb::value::Type::Object, &raw, last.enc, last.timestamp, 0).unwrap();
    let Value::Typed(d) = replica.get("acc:1").unwrap().unwrap().0 else { panic!("typed") };
    assert_eq!(doc::to_extended_json(&d), ext(&inst, "acc:1"));
}

// ---------------------------------------------------------------------------
// Redis, REST, the binary protocol and Lua
// ---------------------------------------------------------------------------

/// One RESP command, its reply as text: `+OK`, `:3`, `$...` bulk text, `-ERR ...`.
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
fn redis_json_commands_change_typed_documents_without_turning_their_values_into_texts() {
    let dir = TempDir::new("nq_redis");
    let inst = open(&dir);
    put(&inst, "acc:1", account());
    let port = kivadb::resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut c = BufReader::new(std::net::TcpStream::connect(("127.0.0.1", port)).unwrap());
    assert_eq!(resp(&mut c, &["JSON.SET", "acc:1", "$.name", r#""Fordima""#]), "+OK");
    assert!(resp(&mut c, &["JSON.NUMINCRBY", "acc:1", ".balance", "1e-1"]).contains("do not mix"));
    assert_eq!(resp(&mut c, &["JSON.NUMINCRBY", "acc:1", ".balance", "0.25"]), "1250.75");
    assert_eq!(resp(&mut c, &["JSON.NUMINCRBY", "acc:1", ".balance", "2"]), "1252.75");
    assert_eq!(resp(&mut c, &["JSON.NUMMULTBY", "acc:1", ".balance", "2"]), "2505.50");
    assert_eq!(resp(&mut c, &["JSON.ARRAPPEND", "acc:1", ".tags", r#""z""#]), ":3");
    assert_eq!(resp(&mut c, &["JSON.ARRINSERT", "acc:1", ".tags", "0", r#""first""#]), ":4");
    assert_eq!(resp(&mut c, &["JSON.DEL", "acc:1", ".profile.opens"]), ":1");
    // Plain JSON for Redis clients: decimals and dates as texts.
    assert_eq!(resp(&mut c, &["JSON.GET", "acc:1", ".balance"]), r#""2505.50""#);
    let all = ext(&inst, "acc:1");
    assert_eq!(all["balance"], json!({"$decimal": "2505.50"}));
    assert_eq!(all["created_at"], json!({"$datetime": "2026-10-31T16:18:55.000Z"}));
    assert_eq!(all["profile"]["born"], json!({"$date": "2004-05-09"}));
    assert_eq!(all["tags"], json!(["first", "a", {"$date": "2027-01-01"}, "z"]));
    assert_eq!(all["name"], json!("Fordima"));
    assert!(all["profile"].get("opens").is_none());
    assert_eq!(enc(&inst, "acc:1"), DocEnc::Kdoc);
}

fn http(port: u16, method: &str, path: &str, body: &str) -> (u16, Json) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
    (status, serde_json::from_str(body).unwrap_or(Json::Null))
}

#[test]
fn rest_and_the_binary_protocol_read_typed_documents_in_plain_json_and_query_them_exactly() {
    let dir = TempDir::new("nq_rest");
    let inst = open(&dir);
    put(&inst, "acc:1", account());
    put(&inst, "acc:2", json!({"balance": {"$decimal": "0.1"}, "since": {"$date": "2026-01-01"}}));
    let port = kivadb::http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let (status, got) = http(port, "GET", "/v1/key/acc:1", "");
    assert_eq!(status, 200);
    assert_eq!(got["data"]["balance"], json!("1250.50"));
    assert_eq!(got["data"]["created_at"], json!("2026-10-31T16:18:55.000Z"));
    let (_, got) = http(port, "GET", "/v1/key/acc:1?path=.profile.born", "");
    assert_eq!(got["data"], json!("2004-05-09"));
    let q = json!({"op": "query", "target": "acc:*", "where": {"field": "balance", "cmp": ">", "value": 1}});
    let (status, got) = http(port, "POST", "/v1/api", &q.to_string());
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["rows"].as_array().map(|r| r.len()), Some(1), "{got}");

    let tcp = kivadb::server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let mut db = kivadb::client::Client::connect(("127.0.0.1", tcp)).unwrap();
    assert_eq!(db.exec("count where acc:*.balance < 1").unwrap(), "Count: 1");
    db.exec(r#"json set acc:2 .note "kept""#).unwrap();
    let Some(Value::Doc(j)) = db.get("acc:2").unwrap() else { panic!("a document") };
    assert_eq!(j, json!({"balance": "0.1", "since": "2026-01-01", "note": "kept"}));
    assert_eq!(ext(&inst, "acc:2")["since"], json!({"$date": "2026-01-01"}), "still a date where it is stored");
}

#[test]
fn lua_reads_decimals_and_dates_as_their_texts() {
    let dir = TempDir::new("nq_lua");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    put(&inst, "acc:1", account());
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("acc:1").balance'"#), "1250.50");
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("acc:1").profile.born'"#), "2004-05-09");
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("acc:1").profile.levels[2][2]'"#), "2.5");
}
