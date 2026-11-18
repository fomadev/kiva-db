// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Extended JSON for programs and the Lua helpers (lot C5 of
//! docs/architecture/NATIVE_DOCUMENTS.md, section 21): plain JSON stays the
//! default everywhere; a request asking for `types: "extended"` reads and
//! writes every type without loss; Lua keeps typed leaves typed.

mod common;

use common::{fails, ok, TempDir};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use kivadb::value::Value;
use serde_json::{json, Value as Json};
use std::io::{Read, Write};
use std::sync::Arc;

fn open(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

fn format_of(dir: &TempDir) -> u32 {
    let h = std::fs::read(dir.path().join("data").join("store.kiva")).unwrap();
    u32::from_le_bytes([h[4], h[5], h[6], h[7]])
}

fn api(s: &mut Session, req: Json) -> Json {
    kivadb::api::handle(s, &req).unwrap_or_else(|e| panic!("{req}: {e}"))
}

fn api_err(s: &mut Session, req: Json) -> String {
    match kivadb::api::handle(s, &req) {
        Ok(j) => panic!("{req} should have failed but answered {j}"),
        Err(e) => e.to_string(),
    }
}

const ACCOUNT: &str = r#"set acc:1 {"name": "Ana", "balance": decimal "1250.50", "opened": date "2026-01-15",
    "at": datetime "2026-10-31T15:18:00Z", "alarm": time "08:30:00", "rate": float 8, "big": 9007199254740993,
    "tags": ["a", date "2027-01-01"]}"#;

fn account_extended() -> Json {
    json!({
        "name": "Ana",
        "balance": {"$decimal": "1250.50"},
        "opened": {"$date": "2026-01-15"},
        "at": {"$datetime": "2026-10-31T15:18:00.000Z"},
        "alarm": {"$time": "08:30:00"},
        "rate": 8.0,
        "big": 9007199254740993_i64,
        "tags": ["a", {"$date": "2027-01-01"}]
    })
}

#[test]
fn structured_requests_answer_plain_json_unless_extended_is_asked() {
    let dir = TempDir::new("ext_get");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, ACCOUNT);

    for plain in [json!({"op": "get", "key": "acc:1"}), json!({"op": "get", "key": "acc:1", "types": "plain"})] {
        let got = api(&mut s, plain);
        assert_eq!(got["type"], "object");
        assert_eq!(got["value"]["balance"], json!("1250.50"));
        assert_eq!(got["value"]["opened"], json!("2026-01-15"));
        assert_eq!(got["value"]["tags"], json!(["a", "2027-01-01"]));
    }
    let got = api(&mut s, json!({"op": "get", "key": "acc:1", "types": "extended"}));
    assert_eq!(got["value"], account_extended());
    assert_eq!(got["value"]["rate"].to_string(), "8.0", "a float keeps its fraction");

    // A key of each type: the kind is named in extended JSON only.
    ok(&mut s, "set n 42");
    ok(&mut s, "set float f 8");
    ok(&mut s, r#"set decimal d "0.10""#);
    ok(&mut s, r#"set date day "2026-03-01""#);
    inst.engine("default").unwrap().put("legacy", &Value::Num("12345678901234567890123".into()), 0, false).unwrap();
    let get = |s: &mut Session, k: &str, types: &str| api(s, json!({"op": "get", "key": k, "types": types}));
    assert_eq!(get(&mut s, "n", "plain")["type"], "number");
    assert_eq!(get(&mut s, "n", "extended")["type"], "int");
    assert_eq!(get(&mut s, "f", "extended")["type"], "float");
    assert_eq!(get(&mut s, "f", "extended")["value"].to_string(), "8.0");
    assert_eq!(get(&mut s, "d", "plain")["value"], json!("0.10"));
    assert_eq!(get(&mut s, "d", "extended")["value"], json!({"$decimal": "0.10"}));
    assert_eq!(get(&mut s, "day", "plain")["value"], json!("2026-03-01"));
    assert_eq!(get(&mut s, "day", "extended")["value"], json!({"$date": "2026-03-01"}));
    assert_eq!(get(&mut s, "legacy", "plain")["value"], json!("12345678901234567890123"), "as before C5");
    assert_eq!(get(&mut s, "legacy", "extended")["value"], json!({"$decimal": "12345678901234567890123"}));
    assert_eq!(get(&mut s, "legacy", "extended")["type"], "number");

    // A plain document: its `$` keys doubled, its legacy numbers tagged.
    inst.engine("default")
        .unwrap()
        .put("plain:1", &Value::from_json(json!({"$ref": 1, "n": 18446744073709551615_u64})), 0, false)
        .unwrap();
    let got = get(&mut s, "plain:1", "extended");
    assert_eq!(got["value"], json!({"$$ref": 1, "n": {"$decimal": "18446744073709551615"}}));

    let e = api_err(&mut s, json!({"op": "get", "key": "n", "types": "extend"}));
    assert!(e.contains(r#""types" is "plain" or "extended""#), "{e}");
    assert!(api_err(&mut s, json!({"op": "get", "key": "n", "types": true})).contains("\"types\""));
}

#[test]
fn extended_writes_keep_their_types_and_plain_writes_stay_plain() {
    let dir = TempDir::new("ext_put");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);

    // Plain JSON never creates a native leaf, even with a tag in it.
    api(&mut s, json!({"op": "put", "key": "p", "type": "object", "value": {"v": {"$decimal": "1"}, "$k": 2}}));
    let got = api(&mut s, json!({"op": "get", "key": "p"}));
    assert_eq!(got["value"], json!({"v": {"$decimal": "1"}, "$k": 2}));
    assert!(format_of(&dir) < 3, "plain writes leave the format as it is");

    // Extended JSON: read back equal, stored with its types.
    api(
        &mut s,
        json!({"op": "put", "key": "acc:1", "type": "object", "value": account_extended(), "types": "extended"}),
    );
    let got = api(&mut s, json!({"op": "get", "key": "acc:1", "types": "extended"}));
    assert_eq!(got["value"], account_extended());
    assert_eq!(ok(&mut s, "json type acc:1 .balance"), "acc:1.balance: [decimal]");
    assert_eq!(format_of(&dir), 3, "a native leaf raises the file");

    let added =
        api(&mut s, json!({"op": "add", "prefix": "e:", "value": {"d": {"$date": "2026-01-01"}}, "types": "extended"}));
    let key = added["key"].as_str().unwrap().to_string();
    assert_eq!(ok(&mut s, &format!("json type {key} .d")), format!("{key}.d: [date]"));

    // `$$` is a `$` key of the data; a lone `$` key is refused.
    api(&mut s, json!({"op": "put", "key": "q", "type": "object", "value": {"$$ref": 1}, "types": "extended"}));
    assert_eq!(api(&mut s, json!({"op": "get", "key": "q"}))["value"], json!({"$ref": 1}));
    let e =
        api_err(&mut s, json!({"op": "put", "key": "q", "type": "object", "value": {"$ref": 1}, "types": "extended"}));
    assert!(e.contains("is not a tag"), "{e}");
    let e = api_err(
        &mut s,
        json!({"op": "put", "key": "q", "type": "object", "value": {"v": {"$date": 1}}, "types": "extended"}),
    );
    assert!(e.contains("takes a text"), "{e}");
    let e = api_err(&mut s, json!({"op": "put", "key": "q", "type": "array", "value": {"a": 1}, "types": "extended"}));
    assert!(e.contains("not a valid array"), "{e}");

    // A typed scalar may come tagged, as it was read.
    api(
        &mut s,
        json!({"op": "put", "key": "d", "type": "decimal", "value": {"$decimal": "1.50"}, "types": "extended"}),
    );
    api(&mut s, json!({"op": "put", "key": "d2", "type": "decimal", "value": "2.50", "types": "extended"}));
    api(
        &mut s,
        json!({"op": "put", "key": "t", "type": "date", "value": {"$date": "2026-05-01"}, "types": "extended"}),
    );
    assert_eq!(ok(&mut s, "get d and d2 and t"), "d: 1.50 [decimal]\nd2: 2.50 [decimal]\nt: 2026-05-01 [date]");
    let e = api_err(
        &mut s,
        json!({"op": "put", "key": "t", "type": "date", "value": {"$decimal": "1"}, "types": "extended"}),
    );
    assert!(e.contains("not a valid date"), "{e}");
}

#[test]
fn a_value_read_in_extended_json_finds_its_document_in_a_condition() {
    let dir = TempDir::new("ext_query");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, ACCOUNT);
    ok(&mut s, r#"set acc:2 {"name": "Ben", "balance": decimal "0.10", "opened": date "2025-01-01"}"#);
    let read = api(&mut s, json!({"op": "get", "key": "acc:1", "types": "extended"}));
    for indexed in [false, true] {
        if indexed {
            ok(&mut s, "create index on acc:*.balance");
        }
        let q = json!({"op": "query", "target": "acc:*", "types": "extended",
            "where": {"field": "balance", "cmp": "=", "value": read["value"]["balance"]}});
        let got = api(&mut s, q);
        let rows = got["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 1, "{got}");
        assert_eq!(rows[0]["value"], account_extended());
    }
    let q = json!({"op": "query", "target": "acc:*",
        "where": {"field": "opened", "cmp": "in", "value": [{"$date": "2025-01-01"}]}});
    let got = api(&mut s, q);
    assert_eq!(got["rows"][0]["key"], "acc:2", "{got}");
    assert_eq!(got["rows"][0]["value"]["balance"], json!("0.10"), "plain rows by default");
}

#[test]
fn the_change_journal_answers_in_both_forms() {
    let dir = TempDir::new("ext_changes");
    let inst = open(&dir);
    let mut s = Session::new(inst, true, None);
    ok(&mut s, "changes enable");
    ok(&mut s, r#"set acc:1 {"balance": decimal "1.50"}"#);
    ok(&mut s, r#"set decimal d "2.50""#);
    let plain = api(&mut s, json!({"op": "changes", "cursor": "beginning"}));
    let ext = api(&mut s, json!({"op": "changes", "cursor": "beginning", "types": "extended"}));
    let values = |j: &Json| j["changes"].as_array().unwrap().iter().map(|c| c["value"].clone()).collect::<Vec<_>>();
    assert_eq!(values(&plain), vec![json!({"balance": "1.50"}), json!("2.50")]);
    assert_eq!(values(&ext), vec![json!({"balance": {"$decimal": "1.50"}}), json!({"$decimal": "2.50"})]);
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
fn rest_reads_and_writes_extended_json_with_types_extended() {
    let dir = TempDir::new("ext_rest");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, ACCOUNT);
    ok(&mut s, "changes enable");
    let port = kivadb::http::start_http(&inst, "127.0.0.1", 0).unwrap();

    let (_, got) = http(port, "GET", "/v1/key/acc:1", "");
    assert_eq!(got["data"]["balance"], json!("1250.50"));
    let (_, got) = http(port, "GET", "/v1/key/acc:1?types=extended", "");
    assert_eq!(got["data"], account_extended());
    let (_, got) = http(port, "GET", "/v1/key/acc:1?path=.opened&types=extended", "");
    assert_eq!(got["data"], json!({"$date": "2026-01-15"}));
    assert_eq!(got["type"], "date");
    let (status, got) = http(port, "GET", "/v1/key/acc:1?types=typed", "");
    assert_eq!(status, 400, "{got}");

    let (status, got) = http(port, "PUT", "/v1/key/price?types=extended", r#"{"$decimal": "9.90"}"#);
    assert_eq!(status, 200, "{got}");
    assert_eq!(got["type"], "decimal");
    let (_, _) =
        http(port, "PUT", "/v1/key/doc?types=extended", r#"{"value": {"d": {"$date": "2026-02-02"}}, "ttl": 0}"#);
    assert_eq!(ok(&mut s, "json type doc .d"), "doc.d: [date]");
    let (_, _) = http(port, "PUT", "/v1/key/plain", r#"{"d": {"$date": "2026-02-02"}}"#);
    assert_eq!(ok(&mut s, "json type plain .d"), "plain.d: [object]");

    let (_, got) = http(port, "GET", "/v1/range?from=price&to=price&types=extended", "");
    assert_eq!(got["data"][0]["data"], json!({"$decimal": "9.90"}), "{got}");
    let (_, got) = http(port, "GET", "/v1/changes?cursor=beginning&types=extended", "");
    let last = got["changes"].as_array().unwrap().iter().rev().find(|c| c["key"] == "price").cloned().unwrap();
    assert_eq!(last["value"], json!({"$decimal": "9.90"}));

    // `?types=` on /v1/api, unless the request says otherwise.
    let (_, got) = http(port, "POST", "/v1/api?types=extended", r#"{"op": "get", "key": "price"}"#);
    assert_eq!(got["value"], json!({"$decimal": "9.90"}));
    let (_, got) = http(port, "POST", "/v1/api?types=extended", r#"{"op": "get", "key": "price", "types": "plain"}"#);
    assert_eq!(got["value"], json!("9.90"));
    // An integer beyond i64 is refused in extended JSON, with the form that keeps it.
    let body =
        r#"{"op": "put", "key": "q", "type": "object", "value": {"v": 12345678901234567890123}, "types": "extended"}"#;
    let (status, got) = http(port, "POST", "/v1/api", body);
    assert_eq!(status, 400, "{got}");
    assert!(got.to_string().contains("$decimal"), "{got}");
}

#[test]
fn lua_keeps_typed_leaves_typed_and_writes_them_with_helpers() {
    let dir = TempDir::new("ext_lua");
    let inst = open(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "timezone UTC");
    ok(&mut s, ACCOUNT);

    // Read, change one field, write back: every type is kept (D4-b).
    ok(&mut s, r#"eval 'local a = kiva.get("acc:1"); a.name = "Anna"; kiva.set("acc:1", a)'"#);
    let got = api(&mut s, json!({"op": "get", "key": "acc:1", "types": "extended"}));
    let mut want = account_extended();
    want["name"] = json!("Anna");
    assert_eq!(got["value"], want);

    // Typed values print and concatenate as their texts.
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("acc:1").balance'"#), "1250.50");
    assert_eq!(ok(&mut s, r#"eval 'return "due " .. kiva.get("acc:1").opened'"#), "due 2026-01-15");
    assert_eq!(
        ok(&mut s, r#"eval 'return tostring(kiva.get("acc:1").at), kiva.get("acc:1").balance:type()'"#),
        "2026-10-31T15:18:00.000Z, decimal"
    );

    // Exact arithmetic and comparisons.
    assert_eq!(ok(&mut s, r#"eval 'return kiva.decimal("0.1") + kiva.decimal("0.2")'"#), "0.3");
    assert_eq!(
        ok(&mut s, r#"eval 'return kiva.decimal("0.1") + 1, kiva.decimal("2.5") * 2, 3 - kiva.decimal("0.5")'"#),
        "1.1, 5.0, 2.5"
    );
    assert_eq!(ok(&mut s, r#"eval 'return kiva.decimal("10.50") == kiva.decimal("10.5")'"#), "true");
    assert_eq!(
        ok(&mut s, r#"eval 'return kiva.date("2026-01-01") < kiva.date("2026-02-01"), kiva.decimal("1") < 2'"#),
        "true, true"
    );
    assert!(fails(&mut s, r#"eval 'return kiva.date("2026-01-01") + 1'"#).contains("date"));

    // Helpers write keys and documents of their types.
    ok(&mut s, r#"eval 'kiva.set("k", kiva.decimal("1.50")); kiva.set("day", kiva.date("2026-03-01"))'"#);
    ok(&mut s, r#"eval 'kiva.set("f", kiva.float(8)); kiva.set("i", kiva.int(8.0))'"#);
    ok(&mut s, r#"eval 'kiva.set("t", kiva.time("08:30:00")); kiva.set("w", kiva.datetime("2026-10-31T15:18:00Z"))'"#);
    assert_eq!(
        ok(&mut s, "get k and day and f and i and t"),
        "k: 1.50 [decimal]\nday: 2026-03-01 [date]\nf: 8.0 [float]\ni: 8 [int]\nt: 08:30:00 [time]"
    );
    assert_eq!(ok(&mut s, "typeof w"), " -> w is a [datetime]");
    ok(&mut s, r#"eval 'kiva.set("doc", {price = kiva.decimal("9.90"), when = {kiva.date("2026-01-01")}})'"#);
    assert_eq!(ok(&mut s, "json type doc .price"), "doc.price: [decimal]");
    assert_eq!(ok(&mut s, "json type doc .when[0]"), "doc.when[0]: [date]");
    assert_eq!(ok(&mut s, r#"eval 'return {a = kiva.decimal("1.0")}'"#), r#"{"a":"1.0"}"#);

    assert!(fails(&mut s, r#"eval 'return kiva.int(2.5)'"#).contains("kiva.int takes an integer"));
    assert!(fails(&mut s, r#"eval 'return kiva.date("31/10/2026")'"#).contains("date"));
    assert!(fails(&mut s, r#"eval 'return kiva.decimal({})'"#).contains("kiva.decimal takes a text"));
}
