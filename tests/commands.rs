mod common;

use common::{fails, instance, ok, run, TempDir};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// What `whoami` tells tools about `compact` in the current workspace.
fn can_compact(s: &mut Session) -> bool {
    kivadb::api::handle(s, &serde_json::json!({"op": "whoami"})).unwrap()["can_compact"] == true
}

fn kiva(dir: &TempDir) -> Session {
    Session::new(instance(dir), true, None)
}

#[test]
fn set_get_update_follow_the_typing_rules() {
    let dir = TempDir::new("typing");
    let mut s = kiva(&dir);
    assert_eq!(ok(&mut s, "set a 4 and string be 'f'"), "OK: a saved.\nOK: be saved.");
    assert_eq!(ok(&mut s, "get a and be"), "a: 4 [number]\nbe: f [string]");
    assert_eq!(ok(&mut s, "get number a"), "a: 4 [number]");
    assert!(fails(&mut s, "get string a").contains("Type mismatch"));
    assert_eq!(ok(&mut s, "get ghost"), "ghost: (nil)");

    assert!(fails(&mut s, "set a 5").contains("already exists"));
    assert!(fails(&mut s, "set name Fordi").contains("must be quoted"));
    assert!(fails(&mut s, "set 44 1").contains("cannot be purely numeric"));
    assert!(fails(&mut s, "set \"quoted\" 1").contains("cannot use quotes"));
    assert!(fails(&mut s, "set get 1").contains("reserved keyword"));
    assert!(fails(&mut s, "set number n \"5\"").contains("must not be quoted"));
    assert!(fails(&mut s, "set string n 5").contains("requires quotes"));
    assert!(fails(&mut s, "set k 1 ttl 0").contains("TTL must be a positive integer"));
    assert!(fails(&mut s, "set k 'unclosed").contains("Unclosed quote"));
    ok(&mut s, "set `get` 1");
    assert_eq!(ok(&mut s, "get `get`"), "get: 1 [number]");

    // One bad segment does not stop the others.
    let out = s.execute("set x 1 and y oops and z 3");
    assert!(out.has_err());
    assert_eq!(
        ok(&mut s, "has x and has y and has z"),
        "Yes: Key 'x' exists.\nNo: Key 'y' does not exist.\nYes: Key 'z' exists."
    );

    assert_eq!(ok(&mut s, "update a 5"), "OK: a updated.");
    assert!(fails(&mut s, "update a \"five\"").contains("Cannot update [number] with a value formatted as [string]"));
    assert!(fails(&mut s, "update ghost 1").contains("not found"));
    assert!(fails(&mut s, "update string a \"x\"").contains("Type mismatch"));
    assert_eq!(ok(&mut s, "get a"), "a: 5 [number]");

    assert_eq!(
        ok(&mut s, "typeof a and be and ghost"),
        " -> a is a [number]\n -> be is a [string]\n -> ghost does not exist (nil)"
    );
    assert_eq!(ok(&mut s, "has number a"), "Yes: Key 'a' exists with type [number].");
    assert!(ok(&mut s, "has string a").starts_with("No: Key 'a' exists but type is [number]"));

    assert!(fails(&mut s, "del string a").contains("Type mismatch"));
    assert_eq!(ok(&mut s, "del a and ghost"), "Deleted: a\nNot found: ghost");
    assert_eq!(ok(&mut s, "del all keys"), "All keys deleted. Database reset.");
    assert_eq!(s.engine.len(), 0);
}

#[test]
fn if_not_exists_and_if_exists_make_commands_idempotent() {
    let dir = TempDir::new("if_exists");
    let mut s = kiva(&dir);

    // set ... if not exists: creates once, then skips without an error.
    assert_eq!(ok(&mut s, "set a 1 if not exists"), "OK: a saved.");
    assert_eq!(ok(&mut s, "set a 2 if not exists"), "OK: a skipped (already exists)");
    assert_eq!(ok(&mut s, "SET a 3 If Not EXISTS"), "OK: a skipped (already exists)");
    assert_eq!(ok(&mut s, "get a"), "a: 1 [number]");
    assert!(fails(&mut s, "set a 4").contains("already exists"));

    // With a type, a TTL and `and` chains (the clause belongs to each segment).
    assert_eq!(ok(&mut s, "set string name 'Fordi' ttl 60 if not exists"), "OK: name saved.");
    assert_eq!(
        ok(&mut s, "set a 9 if not exists and b 2 if not exists"),
        "OK: a skipped (already exists)\nOK: b saved."
    );
    assert!(fails(&mut s, "set a 9 if not exists and b 3").contains("Key 'b' already exists"));
    // The value is still checked, even when the key would be skipped.
    assert!(fails(&mut s, "set c oops if not exists").contains("must be quoted"));
    // A quoted "exists" is a value, not the clause.
    assert!(fails(&mut s, "set d if not \"exists\"").contains("Invalid set syntax"));
    assert!(fails(&mut s, "set if not exists").contains("Missing key or value"));

    // del ... if exists: deletes once, then skips.
    assert_eq!(ok(&mut s, "del a if exists"), "Deleted: a");
    assert_eq!(ok(&mut s, "del a if exists"), "OK: a skipped (does not exist)");
    assert_eq!(ok(&mut s, "DEL a IF EXISTS and b if exists"), "OK: a skipped (does not exist)\nDeleted: b");
    assert!(fails(&mut s, "del number name if exists").contains("Type mismatch"));
    assert_eq!(ok(&mut s, "del string name if exists"), "Deleted: name");

    // Inside a transaction the check sees the transaction's own writes.
    ok(&mut s, "begin");
    assert_eq!(ok(&mut s, "set t 1 if not exists"), "OK: t saved.");
    assert_eq!(ok(&mut s, "set t 2 if not exists"), "OK: t skipped (already exists)");
    assert_eq!(ok(&mut s, "del t if exists"), "Deleted: t");
    assert_eq!(ok(&mut s, "del t if exists"), "OK: t skipped (does not exist)");
    ok(&mut s, "commit");
    assert_eq!(ok(&mut s, "has t"), "No: Key 't' does not exist.");
}

#[test]
fn structured_types() {
    let dir = TempDir::new("structured");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set colors ["red", "green", "blue"]"#);
    ok(
        &mut s,
        r#"set user {"name": "Fordi", "role": "admin", "langs": ["fr", "ln"], "address": {"city": "Kinshasa"}}"#,
    );
    ok(&mut s, "set empty null");
    ok(&mut s, "set raw 0xDEADBEEF and b b64:SGVsbG8=");
    ok(&mut s, r#"set bytes txt "Hello""#);
    ok(&mut s, r#"set object parsed '{"a": 1}'"#);
    assert_eq!(
        ok(&mut s, "typeof colors and user and empty and raw and parsed"),
        " -> colors is a [array]\n -> user is a [object]\n -> empty is a [null]\n -> raw is a [bytes]\n -> parsed is a [object]"
    );
    assert_eq!(
        ok(&mut s, "get raw and b and txt"),
        "raw: 0xdeadbeef [bytes]\nb: 0x48656c6c6f [bytes]\ntxt: 0x48656c6c6f [bytes]"
    );
    assert_eq!(ok(&mut s, "get empty"), "empty: null [null]");
    assert_eq!(ok(&mut s, "get colors"), r#"colors: ["red","green","blue"] [array]"#);
    assert_eq!(ok(&mut s, "get user.name and user.address.city and user.langs[1] and colors.0"),
        "user.name: Fordi [string]\nuser.address.city: Kinshasa [string]\nuser.langs[1]: ln [string]\ncolors.0: red [string]");
    assert_eq!(ok(&mut s, "get user.missing"), "user.missing: (nil)");

    assert!(fails(&mut s, r#"set bad ["unterminated""#).contains("Unclosed bracket"));
    assert!(fails(&mut s, r#"set bad {"a": }"#).contains("Invalid JSON"));
    assert!(fails(&mut s, r#"set array bad {"a": 1}"#).contains("not a JSON array"));
    assert!(fails(&mut s, "update colors 5").contains("Cannot update [array]"));
    ok(&mut s, r#"update colors ["black"]"#);
    // A null key accepts its first real value.
    ok(&mut s, "update empty 12");
    assert_eq!(ok(&mut s, "get empty"), "empty: 12 [number]");

    assert_eq!(ok(&mut s, r#"json set user .role "owner""#), "OK: user.role updated.");
    ok(&mut s, r#"json set user .address.zip 243"#);
    ok(&mut s, r#"json set user .langs[2] "en""#);
    assert_eq!(ok(&mut s, "json get user .role"), "user.role: owner [string]");
    assert_eq!(ok(&mut s, "json get user .address"), r#"user.address: {"city":"Kinshasa","zip":243} [object]"#);
    assert_eq!(ok(&mut s, "json delete user .langs[0]"), "Deleted: user.langs[0]");
    assert_eq!(ok(&mut s, "get user.langs"), r#"user.langs: ["ln","en"] [array]"#);
    assert_eq!(ok(&mut s, "json delete user .nope"), "Not found: user.nope");
    assert!(fails(&mut s, "json set raw .x 1").contains("not an object or array"));

    assert!(ok(&mut s, "convert user to msgpack").contains("converted to MessagePack"));
    assert!(ok(&mut s, "scan").contains("msgpack"));
    assert_eq!(ok(&mut s, "get user.role"), "user.role: owner [string]");
    ok(&mut s, r#"json set user .n 1"#);
    assert!(ok(&mut s, "convert user to msgpack").contains("already stored"));
    assert!(ok(&mut s, "convert user to json").contains("converted to JSON"));
    assert!(fails(&mut s, "convert raw to msgpack").contains("only arrays and objects"));
}

#[test]
fn print_evaluates_expressions() {
    let dir = TempDir::new("print");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set age 30 and name "Fordi" and price 2.5 and u {"n": 4}"#);
    assert_eq!(ok(&mut s, "print (4 + 8) * 10"), "120");
    assert_eq!(ok(&mut s, "print (10 + 5) * 2 / (age - 28)"), "15");
    assert_eq!(ok(&mut s, "print 7 / 2, 7 % 2, -age, price * 2"), "3.5 1 -30 5");
    assert_eq!(ok(&mut s, r#"print "Name:", name, age + u.n"#), "Name: Fordi 34");
    assert_eq!(ok(&mut s, r#"print "User ${name} is ${age + 1}" + "!""#), "User Fordi is 31!");
    assert!(fails(&mut s, "print 1 / 0").contains("division by zero"));
    assert!(fails(&mut s, r#"print "a" + 1"#).contains("cannot concatenate str and int"));
    assert!(fails(&mut s, "print ghost").contains("'ghost' is not defined"));
    assert!(fails(&mut s, "print 1 2").contains("missing separator"));
    assert!(fails(&mut s, r#"print "${ghost}""#).contains("EvaluationError"));
    assert!(fails(&mut s, "print (1 + 2").contains("missing closing parenthesis"));
}

#[test]
fn change_and_bump() {
    let dir = TempDir::new("change");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set active_id 7 and legacy "old" and other 1"#);
    assert_eq!(ok(&mut s, "change active_id to session_id"), "Renamed: active_id -> session_id (Type preserved)");
    assert_eq!(ok(&mut s, "get active_id and session_id"), "active_id: (nil)\nsession_id: 7 [number]");
    assert!(fails(&mut s, "change session_id to other").contains("already exists"));
    assert!(fails(&mut s, "change ghost to x").contains("not found"));
    assert!(fails(&mut s, "change string session_id to x").contains("Type mismatch for source key"));
    assert!(fails(&mut s, "change session_id to string x").contains("Cannot change type without providing a new value"));
    assert_eq!(ok(&mut s, "change string legacy to number code 1044"), "Migrated: legacy -> code (Type: number)");
    assert_eq!(ok(&mut s, "get legacy and code"), "legacy: (nil)\ncode: 1044 [number]");
    // A failed migration leaves the source untouched.
    assert!(fails(&mut s, "change code to boolean flag maybe").contains("has been preserved"));
    assert_eq!(ok(&mut s, "get code and flag"), "code: 1044 [number]\nflag: (nil)");

    ok(&mut s, r#"set sess "x" ttl 100"#);
    assert!(ok(&mut s, "bump sess add 50").contains("add 50s"));
    let ttl = kivadb::util::secs_left(s.engine.meta("sess").unwrap().expires_at);
    assert!((148..=150).contains(&ttl), "ttl was {ttl}");
    ok(&mut s, "bump string sess set 10");
    let ttl = kivadb::util::secs_left(s.engine.meta("sess").unwrap().expires_at);
    assert!((8..=10).contains(&ttl));
    assert!(fails(&mut s, "bump number sess set 10").contains("expected 'number'"));
    assert!(fails(&mut s, "bump ghost set 10").contains("not found"));
    assert!(fails(&mut s, "bump sess mul 10").contains("Use 'add' or 'set'"));
    // Renaming keeps the TTL.
    ok(&mut s, "change sess to sess2");
    assert!(s.engine.meta("sess2").unwrap().expires_at > 0);
}

#[test]
fn queries_ranges_and_indexes() {
    let dir = TempDir::new("queries");
    let mut s = kiva(&dir);
    for i in 1..=30 {
        let role = if i % 5 == 0 { "admin" } else { "dev" };
        ok(
            &mut s,
            &format!(
                r#"set user:{i:03} {{"name": "u{i}", "age": {}, "role": "{role}", "active": {}, "geo": {{"city": "K{}"}}}}"#,
                20 + i,
                i % 2 == 0,
                i % 3
            ),
        );
    }
    ok(&mut s, r#"set zebra "last""#);

    assert_eq!(ok(&mut s, r#"range "user:001" to "user:003""#).lines().count(), 4);
    let out = ok(&mut s, r#"range "user:030" to "user:001" reverse limit 2 return name, age"#);
    assert_eq!(out, "user:030: {\"name\":\"u30\",\"age\":50}\nuser:029: {\"name\":\"u29\",\"age\":49}\n(2 keys)");
    assert!(ok(&mut s, r#"get range "user:001" to "user:100" limit 5"#).ends_with("(5 keys)"));
    assert_eq!(ok(&mut s, "keys limit 2"), "user:001\nuser:002\n(2 keys)");
    assert_eq!(ok(&mut s, r#"keys from "user:029" to "zz""#), "user:029\nuser:030\nzebra\n(3 keys)");
    assert_eq!(ok(&mut s, r#"keys like "user:00?" reverse limit 1"#), "user:009\n(1 keys)");
    assert!(ok(&mut s, r#"scan "z*""#).contains("Total: 1 keys."));

    assert!(ok(&mut s, r#"get where user:*.role = "admin""#).ends_with("(6 matches)"));
    assert_eq!(ok(&mut s, r#"count where user:*.role = "admin""#), "Count: 6");
    assert_eq!(ok(&mut s, "count where user:*.age>=45"), "Count: 6");
    assert_eq!(ok(&mut s, r#"count where "*.active" = true"#), "Count: 15");
    assert_eq!(ok(&mut s, r#"count where user:*.geo.city = "K0""#), "Count: 10");
    assert_eq!(ok(&mut s, "sum where user:*.age > 48"), "Sum: 99 (2 matches)");
    assert_eq!(ok(&mut s, r#"count where user:*.role != "admin""#), "Count: 24");
    assert_eq!(
        ok(&mut s, r#"get where user:*.age < 23 return name, geo.city"#),
        "user:001: {\"name\":\"u1\",\"geo.city\":\"K1\"}\nuser:002: {\"name\":\"u2\",\"geo.city\":\"K2\"}\n(2 matches)"
    );
    assert!(ok(&mut s, r#"get where user:*.role = "admin" limit 2"#).ends_with("(2 matches)"));
    assert!(fails(&mut s, "get where user:*.role = admin").contains("must be quoted"));
    assert!(fails(&mut s, "get where nofield = 1").contains("<pattern>.<field>"));

    let plan = ok(&mut s, r#"explain get where user:*.role = "admin""#);
    assert!(plan.contains("PREFIX SCAN") && plan.contains("Tip       : create index on user:* (role)"));
    assert_eq!(ok(&mut s, "create index on user: (role)"), "Index created on user:* (role): 30 documents indexed.");
    assert!(fails(&mut s, "create index on user: (role)").contains("already exists"));
    let plan = ok(&mut s, r#"explain count where user:*.role = "admin""#);
    assert!(plan.contains("INDEX SCAN") && plan.contains("6 of 31 keys"));
    ok(&mut s, r#"json set user:001 .role "admin""#);
    ok(&mut s, "del user:005");
    assert_eq!(ok(&mut s, r#"count where user:*.role = "admin""#), "Count: 6");
    assert!(ok(&mut s, "indexes").contains("user:* (role) - 29 documents indexed, 2 distinct values"));
    assert!(ok(&mut s, r#"explain range "a" to "z""#).contains("ORDERED INDEX RANGE SCAN"));
    assert!(ok(&mut s, "explain get zebra").contains("POINT LOOKUP"));
    assert_eq!(ok(&mut s, "drop index on user: (role)"), "Index on user:* (role) dropped.");
}

#[test]
fn queries_refuse_joins_and_sorting_pipelines_with_an_explanation() {
    let dir = TempDir::new("query-rules");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set user:1 {"name": "Alice", "role": "dev", "age": 30}"#);
    ok(&mut s, r#"set order:1 {"user": "user:1", "total": 10}"#);
    let cases = [
        (r#"get where user:*.role = "dev" && user:*.age > 20"#, "combined with the words and, or, not"),
        (r#"get where (user:*.role = "dev" or age > 20"#, "missing ')'"),
        (r#"get where user:*.role = "dev")"#, "without its '('"),
        (r#"get where user:*.role = "dev" and order:*.total > 1"#, "fields of the documents of user:*"),
        ("get where user:* join order:* on user", "joins are not supported"),
        ("get where user:*.name = order:*.user", "names a field of other documents"),
        (r#"get where user:*, order:*.role = "dev""#, "joins are not supported"),
        (r#"get where user:*.role = "dev" group by role"#, "sorting pipelines"),
        (r#"get where user:*.role = "dev" limit 5 offset 2"#, "sorting pipelines"),
        (r#"get where user:*.role = "dev" order age"#, "expected 'by'"),
        (r#"get where user:*.role = "dev" order by age reverse"#, "rather than 'reverse'"),
        (r#"count where user:*.role = "dev" group by role order by role"#, "sorting pipelines"),
        (r#"get where user:*.age between 20 and 30"#, "comparison operators"),
        (r#"get where user:*.role in "dev""#, "takes a list of values"),
        (r#"get where user:*.role like 3"#, "quoted text pattern"),
        (r#"get where user:*.role = "dev", "admin""#, "comparison operators"),
        (r#"sum where user:*.age > 20 @node2"#, "distributed queries"),
        (r#"sum where user:*.role = "dev" and age > 1"#, "say which one sum reads"),
        (r#"avg user:* where age > 1"#, "both a key pattern and a field"),
        (r#"count where user:*.role = "dev" group role"#, "expected 'by'"),
        (r#"explain distributed get where user:*.role = "dev""#, "distributed queries"),
    ];
    for (q, why) in cases {
        let err = fails(&mut s, q);
        assert!(err.contains(why), "`{q}`: {err}");
    }
    // What the language does offer still works.
    assert_eq!(
        ok(&mut s, r#"get where user:*.role = "dev" limit 1 return name"#),
        "user:1: {\"name\":\"Alice\"}\n(1 matches)"
    );
    assert!(ok(&mut s, r#"explain get where user:*.age > 20 limit 5"#).contains("PREFIX SCAN"));
    assert_eq!(
        ok(&mut s, r#"get where user:*.role = "dev" limit 5 order by age"#),
        "user:1: {\"name\":\"Alice\",\"role\":\"dev\",\"age\":30}\n(1 matches)"
    );

    let q = |extra: serde_json::Value| {
        let mut r = serde_json::json!({"op": "query", "target": "user:*.role", "cmp": "=", "value": "dev"});
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    assert!(kivadb::api::handle(&mut s, &q(serde_json::json!({}))).is_ok());
    for (extra, why) in [
        (serde_json::json!({"join": "order:*"}), "joins"),
        (serde_json::json!({"order_by": "age sideways"}), "neither asc nor desc"),
        (serde_json::json!({"offset": 2}), "sorting pipelines"),
        (serde_json::json!({"and": [{"target": "user:*.age"}]}), "\"where\""),
        (serde_json::json!({"where": {"age": 1}}), "not both"),
        (serde_json::json!({"value": ["dev", "admin"]}), "not with an array"),
        (serde_json::json!({"target": "user:*.ro*"}), "wildcard"),
    ] {
        let err = kivadb::api::handle(&mut s, &q(extra)).unwrap_err().to_string();
        assert!(err.contains(why), "{err}");
    }
}

#[test]
fn pinned_values_stay_in_memory_and_the_memory_bound_refuses_new_keys() {
    let dir = TempDir::new("memory");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set conf:a {"x": 1} and conf:b "on" and other 1"#);
    assert!(ok(&mut s, r#"memory pin "conf:*""#).starts_with("Pinned 'conf:*': 2 values"));
    assert!(fails(&mut s, r#"memory pin "conf:*""#).contains("already pinned"));
    let status = ok(&mut s, "memory");
    assert!(status.contains("Pinned values     :") && status.contains("(2 values)"), "{status}");
    assert!(status.contains("Pinned patterns   : conf:*"), "{status}");
    // New and changed keys under the pattern are kept in memory too.
    ok(&mut s, r#"set conf:c 3"#);
    ok(&mut s, r#"json set conf:a .x 2"#);
    assert!(ok(&mut s, "memory").contains("(3 values)"));
    ok(&mut s, "del conf:c");
    assert!(ok(&mut s, "memory").contains("(2 values)"));
    assert_eq!(ok(&mut s, "get conf:a.x"), "conf:a.x: 2 [number]");

    // Pinned patterns are saved with the workspace.
    drop(s);
    let mut s = kiva(&dir);
    let status = ok(&mut s, "memory");
    assert!(status.contains("(2 values)") && status.contains("conf:*"), "{status}");
    assert!(ok(&mut s, r#"memory unpin "conf:*""#).starts_with("Unpinned"));
    assert!(fails(&mut s, r#"memory unpin "conf:*""#).contains("not pinned"));
    assert!(ok(&mut s, "memory").contains("(0 values)"));

    // The bound refuses new keys only; existing keys can change and be deleted.
    ok(&mut s, "config set max_memory_mb 1");
    let mut refused = None;
    for batch in 0..100 {
        let cmd: Vec<String> = (0..100).map(|i| format!("set key:{batch:03}:{i:03} {i}")).collect();
        let out = s.execute(&cmd.join(" and "));
        if out.has_err() {
            refused = Some(out.text());
            break;
        }
    }
    let refused = refused.expect("1 MB should not hold 10 000 keys");
    assert!(refused.contains("OOM:") && refused.contains("new keys are refused"), "{refused}");
    assert!(ok(&mut s, "memory").contains("Bound             : 1.00 MB"));
    assert!(fails(&mut s, "set brand:new 1").contains("OOM:"));
    assert!(fails(&mut s, r#"memory pin "key:*""#).contains("OOM:"));
    assert_eq!(ok(&mut s, "update key:000:000 7"), "OK: key:000:000 updated.");
    ok(&mut s, r#"del key:000:000 and key:000:001 and key:000:002 and key:000:003 and key:000:004"#);
    assert_eq!(ok(&mut s, "set brand:new 1"), "OK: brand:new saved.");
    ok(&mut s, "config set max_memory_mb 0");
    ok(&mut s, "set another:one 1");
}

#[test]
fn redis_clients_reach_vector_search_and_the_semantic_cache() {
    use kivadb::resp::{Conn, Reply};
    let dir = TempDir::new("resp-vectors");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set doc:1 {"title": "Kinshasa guide", "embedding": [0.9, 0.1, 0.0]}"#);
    ok(&mut s, r#"set doc:2 {"title": "Goma guide", "embedding": [0.1, 0.9, 0.0]}"#);
    ok(&mut s, r#"set doc:3 {"title": "Recipes", "embedding": [0.0, 0.1, 0.9]}"#);
    let mut c = Conn::new(Session::new(inst, true, None));
    let call = |c: &mut Conn, args: &[&str]| c.handle(&args.iter().map(|a| a.as_bytes().to_vec()).collect::<Vec<_>>());

    let Reply::Array(Some(rows)) = call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[1, 0, 0]", "LIMIT", "2"]) else {
        panic!("not an array");
    };
    assert_eq!(rows.len(), 2);
    let Reply::Array(Some(first)) = &rows[0] else { panic!() };
    assert_eq!(first[0], Reply::Bulk(Some(b"doc:1".to_vec())));
    assert_eq!(first[1], Reply::Double(0.9938837));
    let Reply::Array(Some(second)) = &rows[1] else { panic!() };
    assert_eq!(second[1], Reply::Double(0.11043153));
    let Reply::Bulk(Some(doc)) = &first[2] else { panic!() };
    assert!(String::from_utf8_lossy(doc).contains("Kinshasa guide"));
    assert!(
        matches!(call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[1, 0, 0]", "METRIC", "l2", "EXACT"]), Reply::Array(Some(r)) if r.len() == 3)
    );
    assert!(
        matches!(call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[oops"]), Reply::Error(e) if e.contains("JSON array"))
    );
    // Text instead of a vector: the embedding is the application's to compute.
    assert!(
        matches!(call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "a guide to Kinshasa"]), Reply::Error(e) if e.contains("does not compute embeddings"))
    );
    // FILTER: conditions on the documents' other fields, all of which hold.
    let Reply::Array(Some(rows)) =
        call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[1, 0, 0]", "FILTER", "title", "=", "Goma guide"])
    else {
        panic!("not an array");
    };
    assert_eq!(rows.len(), 1);
    assert!(matches!(&rows[0], Reply::Array(Some(r)) if matches!(&r[0], Reply::Bulk(Some(k)) if k == b"doc:2")));
    assert!(matches!(
        call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[1, 0, 0]", "FILTER", "title", "~", "x"]),
        Reply::Error(e) if e.contains("cmp must be")
    ));
    assert!(
        matches!(call(&mut c, &["KIVA.SIMILAR", "doc:*.embedding", "[1,0,0]", "LIMIT"]), Reply::Error(e) if e.contains("needs a value"))
    );

    // Any structured request: here the semantic cache of an LLM application.
    let store =
        r#"{"op": "semantic", "action": "store", "cache": "llm", "text": "What is KivaDB?", "answer": "A database."}"#;
    assert!(
        matches!(call(&mut c, &["KIVA.API", store]), Reply::Bulk(Some(b)) if String::from_utf8_lossy(&b).contains("\"id\""))
    );
    let lookup = r#"{"op": "semantic", "cache": "llm", "text": "What is KivaDB?"}"#;
    let Reply::Bulk(Some(hit)) = call(&mut c, &["KIVA.API", lookup]) else { panic!() };
    assert!(String::from_utf8_lossy(&hit).contains("A database."), "{}", String::from_utf8_lossy(&hit));
    assert!(matches!(call(&mut c, &["KIVA.API", "not json"]), Reply::Error(e) if e.contains("JSON object")));
    assert!(matches!(call(&mut c, &["KIVA.API", r#"{"op": "nope"}"#]), Reply::Error(_)));
}

#[test]
fn json_changes_arrays_numbers_and_awkward_paths_in_place() {
    let dir = TempDir::new("json-ops");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set doc {"tags": ["a", "b"], "n": 1, "m": {"x": 1}}"#);
    assert_eq!(ok(&mut s, "json get doc .tags[-1]"), "doc.tags[-1]: b [string]");
    assert_eq!(ok(&mut s, r#"json append doc .tags "c", "d""#), "OK: doc.tags now has 4 elements.");
    assert_eq!(
        ok(&mut s, r#"json insert doc .tags[0] "z""#),
        "OK: inserted at doc.tags[0]; the array now has 5 elements."
    );
    assert_eq!(ok(&mut s, r#"json append doc .new 1"#), "OK: doc.new now has 1 elements.");
    assert_eq!(ok(&mut s, "json delete doc .tags[-1]"), "Deleted: doc.tags[-1]");
    assert_eq!(ok(&mut s, "json get doc .tags"), r#"doc.tags: ["z","a","b","c"] [array]"#);
    assert_eq!(ok(&mut s, "json incr doc .n 41"), "doc.n: 42 [number]");
    assert_eq!(ok(&mut s, "json incr doc .n -0.5"), "doc.n: 41.5 [number]");
    assert_eq!(ok(&mut s, "json incr doc .stats.views 1"), "doc.stats.views: 1 [number]");
    assert_eq!(ok(&mut s, r#"json set doc '["a.b"]' true"#), r#"OK: doc["a.b"] updated."#);
    assert_eq!(ok(&mut s, r#"json get doc ["a.b"]"#), r#"doc["a.b"]: true [boolean]"#);
    assert_eq!(ok(&mut s, "json type doc .m"), "doc.m: [object]");
    assert_eq!(ok(&mut s, "json len doc .tags"), "doc.tags: 4");
    assert_eq!(ok(&mut s, "json keys doc .m"), "x");
    assert_eq!(ok(&mut s, "json len doc"), "doc: 6");
    assert_eq!(ok(&mut s, "json get doc .nothing"), "doc.nothing: (nil)");

    assert!(fails(&mut s, "json incr doc .tags 1").contains("not a number (doc.tags)"));
    assert!(fails(&mut s, r#"json incr doc .n "1""#).contains("must be a number"));
    assert!(fails(&mut s, "json append doc .n 1").contains("not an array"));
    assert!(fails(&mut s, "json insert doc .tags[9] 1").contains("out of range"));
    assert!(fails(&mut s, "json get doc .tags..x").contains("empty segment"));
    assert!(fails(&mut s, "json len doc .n").contains("no length"));
    assert!(fails(&mut s, "json incr ghost .n 1").contains("not found"));

    // Inside a transaction the change waits for the commit.
    ok(&mut s, "begin");
    ok(&mut s, "json incr doc .stats.views 1");
    assert_eq!(ok(&mut s, "json get doc .stats.views"), "doc.stats.views: 2 [number]");
    ok(&mut s, "rollback");
    assert_eq!(ok(&mut s, "json get doc .stats.views"), "doc.stats.views: 1 [number]");
}

#[test]
fn field_changes_from_two_clients_never_undo_each_other() {
    let dir = TempDir::new("json-race");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set counter {"a": 0, "b": 0}"#);
    let workers: Vec<_> = ["a", "b"]
        .into_iter()
        .map(|field| {
            let inst = inst.clone();
            std::thread::spawn(move || {
                let mut s = Session::new(inst, true, None);
                for _ in 0..300 {
                    ok(&mut s, &format!("json incr counter .{field} 1"));
                    ok(&mut s, &format!(r#"json set counter .last_{field} "x""#));
                }
            })
        })
        .collect();
    for w in workers {
        w.join().unwrap();
    }
    assert_eq!(ok(&mut s, "json get counter .a"), "counter.a: 300 [number]");
    assert_eq!(ok(&mut s, "json get counter .b"), "counter.b: 300 [number]");
}

#[test]
fn hash_indexes_and_the_dotted_index_syntax() {
    let dir = TempDir::new("hash-index");
    let mut s = kiva(&dir);
    for i in 1..=20 {
        let city = ["Kinshasa", "Goma", "Lubumbashi", "Bukavu"][i % 4];
        ok(&mut s, &format!(r#"set user:{i:02} {{"age": {}, "address": {{"city": "{city}"}}}}"#, 20 + i));
    }
    assert_eq!(
        ok(&mut s, "create index on user:*.address.city using hash"),
        "Index created on user:* (address.city) using hash: 20 documents indexed."
    );
    assert_eq!(
        ok(&mut s, "create index on user:(age) using btree"),
        "Index created on user:* (age): 20 documents indexed."
    );
    assert!(fails(&mut s, "create index on user: (age) using hash").contains("already exists (btree)"));
    assert!(fails(&mut s, "create index on user: (x) using trie").contains("btree or hash"));
    let list = ok(&mut s, "indexes");
    assert!(list.contains("user:* (address.city) - hash, 20 documents indexed, 4 distinct values"), "{list}");
    assert!(list.contains("user:* (age) - 20 documents indexed, 20 distinct values"), "{list}");

    let plan = ok(&mut s, r#"explain get where user:*.address.city = "Goma""#);
    assert!(plan.contains("INDEX SCAN") && plan.contains("using hash") && plan.contains("5 of 20"), "{plan}");
    // A hash index cannot answer a range: it falls back to a scan.
    ok(&mut s, "drop index on user:*.age");
    let plan = ok(&mut s, r#"explain get where user:*.address.city > "A""#);
    assert!(plan.contains("PREFIX SCAN"), "{plan}");

    // Kept exact by field changes, deletes and restarts.
    ok(&mut s, r#"json set user:02 .address.city "Goma""#);
    ok(&mut s, "del user:05");
    assert_eq!(ok(&mut s, r#"count where user:*.address.city = "Goma""#), "Count: 5");
    drop(s);
    let mut s = kiva(&dir);
    assert!(ok(&mut s, "indexes").contains("user:* (address.city) - hash, 19 documents indexed"));
    assert_eq!(ok(&mut s, r#"count where user:*.address.city = "Goma""#), "Count: 5");
    assert!(ok(&mut s, r#"explain count where user:*.address.city = "Goma""#).contains("using hash"));
}

#[test]
fn numbers_are_compared_and_added_exactly() {
    let dir = TempDir::new("exact-numbers");
    let mut s = kiva(&dir);
    // 2^53 and the integers around it are one float, but distinct values.
    let values =
        ["9007199254740991", "9007199254740992", "9007199254740993", "9007199254740992.0", "-9007199254740993"];
    for (i, v) in values.iter().enumerate() {
        ok(&mut s, &format!(r#"set n:{i} {{"v": {v}}}"#));
    }
    let queries = [
        "n:*.v = 9007199254740993",
        "n:*.v > 9007199254740992",
        "n:*.v >= 9007199254740992",
        "n:*.v < -9007199254740992",
        "n:*.v in [9007199254740991, 9007199254740993]",
    ];
    let scan: Vec<String> = queries.iter().map(|q| ok(&mut s, &format!("get where {q}"))).collect();
    assert_eq!(scan[0], "n:2: {\"v\":9007199254740993}\n(1 matches)");
    assert_eq!(scan[1], "n:2: {\"v\":9007199254740993}\n(1 matches)");
    assert!(scan[2].ends_with("(3 matches)"), "2^53 twice, and 2^53 + 1: {}", scan[2]);
    assert_eq!(scan[3], "n:4: {\"v\":-9007199254740993}\n(1 matches)");
    assert!(scan[4].ends_with("(2 matches)"));
    // An index answers as the scan does.
    ok(&mut s, "create index on n:*.v");
    for (q, before) in queries.iter().zip(&scan) {
        assert_eq!(&ok(&mut s, &format!("get where {q}")), before, "{q}");
    }
    let sorted = ok(&mut s, "get where n:*.v > 0 order by v desc limit 1");
    assert!(sorted.starts_with("n:2:"), "{sorted}");
    assert_eq!(ok(&mut s, "sum n:*.v where v > 0"), "Sum: 36028797018963968 (4 matches)");
    assert_eq!(ok(&mut s, "json incr n:1 .v 1"), "n:1.v: 9007199254740993 [number]");

    // A unique index tells them apart, and still takes 1 and 1.0 for one value.
    ok(&mut s, "create unique index on u:*.v");
    ok(&mut s, r#"set u:1 {"v": 9007199254740992}"#);
    ok(&mut s, r#"set u:2 {"v": 9007199254740993}"#);
    ok(&mut s, r#"set u:3 {"v": 1.0}"#);
    assert!(fails(&mut s, r#"set u:4 {"v": 1}"#).contains("UNIQUE"));

    // An integer that would be rounded is refused, not rounded.
    let big = "12345678901234567890123";
    assert!(fails(&mut s, &format!(r#"set b:1 {{"v": {big}}}"#)).contains("beyond the 64-bit integers"));
    assert!(fails(&mut s, &format!("get where n:*.v = {big}")).contains("would be rounded"));
    ok(&mut s, &format!(r#"set b:2 {{"v": "{big}", "f": 1e30}}"#));
    ok(&mut s, r#"set r:1 {"v": 9223372036854775807}"#);
    ok(&mut s, r#"set r:2 {"v": 9223372036854775807}"#);
    assert_eq!(ok(&mut s, "sum r:*.v"), "Sum: 18446744073709551614 (2 matches)");
    ok(&mut s, r#"set r:3 {"v": 9223372036854775807}"#);
    assert!(fails(&mut s, "sum r:*.v").contains("beyond the 64-bit integers"));

    // A key keeps its text; programs get it exactly, as a number or as that text.
    ok(&mut s, &format!("set big {big}"));
    ok(&mut s, "set price 1250.50");
    let get = |s: &mut Session, k: &str| kivadb::api::handle(s, &serde_json::json!({"op": "get", "key": k})).unwrap();
    assert_eq!(get(&mut s, "big")["value"], serde_json::json!(big));
    assert_eq!(get(&mut s, "price")["value"], serde_json::json!(1250.5));
    assert!(fails(&mut s, "print big").contains("cannot be held exactly"));
    assert_eq!(ok(&mut s, "print price + 1"), "1251.5");

    // A schema's maximum is checked exactly.
    ok(&mut s, r#"schema set m: {"v": {"type": "integer", "max": 9007199254740992}}"#);
    ok(&mut s, r#"set m:1 {"v": 9007199254740992}"#);
    assert!(fails(&mut s, r#"set m:2 {"v": 9007199254740993}"#).contains("above the maximum"));
}

#[test]
fn transactions() {
    let dir = TempDir::new("txn");
    let inst = instance(&dir);
    let mut a = Session::new(inst.clone(), true, None);
    let mut b = Session::new(inst.clone(), true, None);
    ok(&mut a, "set balance 100 and other 1");

    ok(&mut a, "begin");
    ok(&mut a, "update balance 50");
    ok(&mut a, r#"set user:1 "Alice""#);
    ok(&mut a, "del other");
    // Read-your-writes inside, invisible outside.
    assert_eq!(
        ok(&mut a, "get balance and user:1 and other"),
        "balance: 50 [number]\nuser:1: Alice [string]\nother: (nil)"
    );
    assert_eq!(
        ok(&mut b, "get balance and user:1 and other"),
        "balance: 100 [number]\nuser:1: (nil)\nother: 1 [number]"
    );
    ok(&mut a, "savepoint sp1");
    ok(&mut a, r#"set user:2 "Bob""#);
    assert!(ok(&mut a, "transaction status").contains("Savepoints: 1\nModified keys: 4"));
    ok(&mut a, "rollback to sp1");
    assert!(fails(&mut a, "rollback to nope").contains("does not exist"));
    assert_eq!(ok(&mut a, "commit"), "Transaction committed (3 keys written).");
    assert_eq!(
        ok(&mut b, "get balance and user:1 and user:2 and other"),
        "balance: 50 [number]\nuser:1: Alice [string]\nuser:2: (nil)\nother: (nil)"
    );

    // Explicit rollback.
    ok(&mut a, "begin");
    ok(&mut a, "update balance 0");
    assert_eq!(ok(&mut a, "rollback"), "Transaction rolled back.");
    assert_eq!(ok(&mut a, "get balance"), "balance: 50 [number]");
    assert!(fails(&mut a, "commit").contains("no active transaction"));
    assert!(ok(&mut a, "transaction status").contains("NONE"));

    // An error inside a transaction rolls everything back.
    ok(&mut a, "begin");
    ok(&mut a, "update balance 1");
    assert!(fails(&mut a, "update balance \"text\"").contains("Transaction rolled back"));
    assert!(a.txn.is_none());
    assert_eq!(ok(&mut a, "get balance"), "balance: 50 [number]");

    // Two sessions touching the same key: the second commit loses.
    ok(&mut a, "begin");
    ok(&mut b, "begin");
    ok(&mut a, "update balance 10");
    ok(&mut b, "update balance 20");
    ok(&mut b, r#"set side "effect""#);
    ok(&mut a, "commit");
    assert!(fails(&mut b, "commit").contains("ERR_TRANSACTION_CONFLICT"));
    assert_eq!(ok(&mut b, "get balance and side"), "balance: 10 [number]\nside: (nil)");
}

#[test]
fn committed_transactions_survive_a_restart() {
    let dir = TempDir::new("txn_restart");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "begin");
        ok(&mut s, r#"set a 1 and b "two" and c [3]"#);
        ok(&mut s, "commit");
        ok(&mut s, "begin");
        ok(&mut s, "set never 1");
        drop(s);
        inst.shutdown();
    }
    let mut s = kiva(&dir);
    assert_eq!(
        ok(&mut s, "get a and b and c and never"),
        "a: 1 [number]\nb: two [string]\nc: [3] [array]\nnever: (nil)"
    );
    assert!(ok(&mut s, "stats").contains("from hint file"));
}

#[test]
fn users_roles_and_workspaces() {
    let dir = TempDir::new("users");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, r#"set shared "team data""#);
    ok(&mut admin, r#"create user "fordi" identified by "s3cret""#);
    ok(&mut admin, r#"create user "guest" identified by "g" role viewer"#);
    assert!(fails(&mut admin, r#"create user "fordi" identified by "x""#).contains("already exists"));
    assert!(fails(&mut admin, r#"create user "bad name" identified by "x""#).contains("user names may only contain"));
    ok(&mut admin, "config set auth_required true");

    // From now on a new session starts unauthenticated.
    let mut s = Session::new(inst.clone(), false, None);
    assert!(fails(&mut s, "get shared").contains("authentication required"));
    assert!(fails(&mut s, "auth fordi wrong").contains("invalid username or password"));
    assert!(fails(&mut s, "auth nobody x").contains("invalid username or password"));
    // Root still has an empty password: refused over the network, allowed locally.
    assert!(fails(&mut s, "auth kiva").contains("only log in from the local shell"));
    assert_eq!(ok(&mut s, "auth fordi s3cret"), "Welcome fordi (editor)");
    // A new account reaches nothing until it is given access.
    assert_eq!(ok(&mut s, "whoami"), "fordi (editor), in no workspace");
    assert!(fails(&mut s, "get shared").contains("you are in no workspace"));
    assert!(fails(&mut s, "set mine 1").contains("you are in no workspace"));
    assert_eq!(ok(&mut s, "workspace list"), "");
    assert!(fails(&mut s, "workspace switch default").contains("no access"));

    // A project groups workspaces; write on all of them makes fordi its manager.
    ok(&mut admin, "project create travel");
    ok(&mut admin, "workspace create travel.places");
    ok(&mut admin, "workspace create travel.cache");
    ok(&mut admin, r#"grant write on travel.* to "fordi""#);
    let list = ok(&mut s, "workspace list");
    assert!(list.contains("travel.places") && list.contains("travel.cache") && !list.contains("default"), "{list}");
    ok(&mut s, "workspace switch travel.places");
    assert_eq!(ok(&mut s, "get shared"), "shared: (nil)");
    ok(&mut s, r#"set mine "private""#);
    // The project's maintenance is allowed, and new workspaces in it; the server's, not.
    // Tools are told so beforehand.
    assert!(can_compact(&mut s));
    assert!(ok(&mut s, "compact").contains("compacted successfully"));
    ok(&mut s, "workspace create travel.logs");
    assert!(fails(&mut s, "workspace create logs").contains("outside a project requires the admin role"));
    assert!(fails(&mut s, "workspace create school.logs").contains("project 'school' does not exist"));
    assert!(fails(&mut s, "project create school").contains("requires the admin role"));
    assert!(fails(&mut s, "del all keys").contains("requires the admin role"));
    assert!(fails(&mut s, r#"create user "x" identified by "y""#).contains("requires the admin role"));
    assert!(fails(&mut s, "config list").contains("requires the admin role"));
    assert!(fails(&mut s, r#"modify user "guest" password "hacked""#).contains("only change your own password"));
    assert!(fails(&mut s, r#"modify user "fordi" role "admin""#).contains("requires the admin role"));

    ok(&mut admin, r#"grant read on workspace "default" to "fordi""#);
    ok(&mut s, "workspace switch default");
    assert_eq!(ok(&mut s, "get shared"), "shared: team data [string]");
    assert!(fails(&mut s, "set nope 1").contains("only have read access"));
    let list = ok(&mut s, "workspace list");
    assert!(list.contains("default") && list.contains("travel.logs"));
    ok(&mut admin, r#"grant write on all to "fordi""#);
    ok(&mut s, "workspace switch default");
    ok(&mut s, "set yes 1");
    // Writing somewhere outside a project does not make it one's own: compacting it stays an admin task.
    assert!(!can_compact(&mut s));
    assert!(fails(&mut s, "compact").contains("requires the admin role"));
    ok(&mut admin, r#"revoke write on *.* from "fordi""#);
    ok(&mut s, "workspace switch default");
    assert!(fails(&mut s, "set again 1").contains("only have read access"));

    // Viewers can read but never write.
    let mut g = Session::new(inst.clone(), false, None);
    ok(&mut admin, r#"grant read on travel.places to "guest""#);
    // The only workspace guest can reach is the one a login enters.
    ok(&mut g, "auth guest g");
    assert_eq!(ok(&mut g, "whoami"), "guest (viewer) @ workspace 'travel.places'");
    assert!(fails(&mut g, "set k 1").contains("viewer role is read-only"));
    assert!(fails(&mut g, "eval 'return 1'").contains("viewer role is read-only"));
    assert!(fails(&mut g, "compact").contains("viewer role is read-only"));
    ok(&mut admin, r#"modify user "guest" role "editor""#);
    ok(&mut admin, r#"grant write on travel.places to "guest""#);
    ok(&mut g, "auth guest g");
    ok(&mut g, "set k 1");
    ok(&mut admin, r#"revoke write from "guest""#);
    ok(&mut g, "auth guest g");
    assert!(fails(&mut g, "set k2 1").contains("read-only"));

    // Password changes.
    ok(&mut s, r#"modify user "fordi" password "n3w""#);
    let mut again = Session::new(inst.clone(), false, None);
    assert!(fails(&mut again, "auth fordi s3cret").contains("invalid"));
    ok(&mut again, "auth fordi n3w");
    ok(&mut admin, r#"modify user "kiva" password "rootpw""#);
    assert_eq!(ok(&mut again, "auth kiva rootpw"), "Welcome kiva (admin)");

    assert!(fails(&mut admin, r#"drop user "kiva""#).contains("cannot be dropped"));
    ok(&mut admin, r#"drop user "guest""#);
    assert!(fails(&mut g, "auth guest g").contains("invalid"));
    let users = ok(&mut admin, "users");
    assert!(users.contains("fordi") && users.contains("kiva") && !users.contains("guest"));

    // Everything sensitive is in the audit log, without the passwords.
    let audit = ok(&mut admin, "audit last 100");
    assert!(audit.contains("create user 'fordi' identified by '***'"));
    assert!(!audit.contains("s3cret") && !audit.contains("n3w") && !audit.contains("rootpw"));
    assert!(audit.contains("user=fordi action=\"compact\" status=SUCCESS"));
    assert!(audit.contains("user=fordi action=\"compact\" status=FAILURE"));
    assert!(ok(&mut admin, "audit failed-logins").contains("login (wrong password)"));
    assert!(ok(&mut admin, r#"audit since "2020-01-01""#).lines().count() > 5);
    assert_eq!(ok(&mut admin, r#"audit since "2999-01-01""#), "No audit entries.");
}

#[test]
fn projects_group_workspaces_and_grants_name_them() {
    let dir = TempDir::new("projects");
    let inst = instance(&dir);
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, "project create travel");
    ok(&mut admin, "project create school");
    assert!(fails(&mut admin, "project create travel").contains("already exists"));
    assert!(fails(&mut admin, "project create b.a").contains("project names may only contain"));
    assert!(fails(&mut admin, "workspace create shop.places").contains("project 'shop' does not exist"));
    assert!(fails(&mut admin, "workspace create a.b.c").contains("workspace names may only contain"));
    for w in ["travel.places", "travel.cache", "school.places"] {
        ok(&mut admin, &format!("workspace create {w}"));
    }

    // The same name in two projects: two workspaces, nothing shared.
    ok(&mut admin, "workspace switch travel.places");
    ok(&mut admin, r#"set place:1 {"name": "Goma"}"#);
    ok(&mut admin, "workspace switch school.places");
    assert_eq!(ok(&mut admin, "get place:1"), "place:1: (nil)");
    let travel = ok(&mut admin, "workspace list travel");
    assert!(
        travel.contains("travel.places") && travel.contains("travel.cache") && !travel.contains("school.places"),
        "{travel}"
    );

    // One command creates the account and gives what it may reach.
    assert_eq!(
        ok(
            &mut admin,
            r#"create user "app" identified by "pw" grant write on travel.*, school.places grant read on default default workspace travel.places"#
        ),
        "User 'app' created with role [editor]: read default, write school.places, write travel.*."
    );
    assert!(
        ok(&mut admin, "users").contains("default=read, school.places=write, travel.*=write (default: travel.places)")
    );
    assert!(fails(&mut admin, r#"create user "x" identified by "y" grant write on shop.*"#)
        .contains("project 'shop' does not exist"));
    assert!(fails(&mut admin, r#"create user "x" identified by "y" grant write on travel.nowhere"#)
        .contains("does not exist"));
    assert!(fails(&mut admin, r#"create user "x" identified by "y" grant write on a.b.c"#).contains("is not a scope"));
    assert!(fails(&mut admin, r#"grant write on travel.* school.* to "app""#).contains("Usage"));
    ok(&mut admin, "config set auth_required true");

    let mut app = Session::new(inst.clone(), false, None);
    ok(&mut app, "auth app pw");
    assert_eq!(ok(&mut app, "whoami"), "app (editor) @ workspace 'travel.places'");
    assert_eq!(ok(&mut app, "get place:1"), r#"place:1: {"name":"Goma"} [object]"#);
    ok(&mut app, "workspace switch school.places");
    ok(&mut app, "set paid 1");
    // user@workspace logs in straight into that workspace, as Redis URLs can say.
    let mut direct = Session::new(inst.clone(), false, None);
    ok(&mut direct, "auth app@school.places pw");
    assert_eq!(ok(&mut direct, "get paid"), "paid: 1 [number]");
    let mut lost = Session::new(inst.clone(), false, None);
    assert!(fails(&mut lost, "auth app@travel.nowhere pw").contains("does not exist"));
    assert!(fails(&mut lost, "whoami").contains("authentication required"));
    assert!(ok(&mut app, "project list").contains("travel               2 workspace(s): cache, places"));
    // A grant on a whole project covers its workspaces created later.
    ok(&mut admin, "workspace create travel.logs");
    ok(&mut app, "workspace switch travel.logs");
    ok(&mut app, "set line 1");
    // Grants on everything named places, in every project.
    ok(&mut admin, r#"create user "reader" identified by "pw" role viewer grant read on *.places"#);
    let mut r = Session::new(inst.clone(), false, None);
    ok(&mut r, "auth reader pw");
    assert!(ok(&mut r, "whoami").contains("in no workspace"));
    let list = ok(&mut r, "workspace list");
    assert!(list.contains("travel.places") && list.contains("school.places") && !list.contains("cache"), "{list}");

    // A workspace moves into a project; its data and grants follow it.
    ok(&mut admin, "workspace create old_places");
    ok(&mut admin, "workspace switch old_places");
    ok(&mut admin, "set kept 1");
    ok(&mut admin, r#"grant read on old_places to "reader""#);
    assert!(fails(&mut admin, "workspace move old_places to travel.archive").contains("the workspace you are in"));
    ok(&mut admin, "workspace switch default");
    ok(&mut r, "workspace switch old_places");
    assert!(fails(&mut admin, "workspace move old_places to travel.archive").contains("is in use"));
    // A move refused leaves the grants as they were: none on the new name.
    let users = ok(&mut admin, "users");
    assert!(users.contains("old_places=read") && !users.contains("travel.archive"), "{users}");
    ok(&mut r, "workspace switch travel.places");
    assert!(fails(&mut admin, "workspace move old_places to shop.archive").contains("project 'shop' does not exist"));
    assert!(fails(&mut admin, "workspace move old_places to travel.places").contains("already exists"));
    ok(&mut admin, "workspace move old_places to travel.archive");
    ok(&mut r, "workspace switch travel.archive");
    assert_eq!(ok(&mut r, "get kept"), "kept: 1 [number]");
    assert!(fails(&mut r, "workspace switch old_places").contains("does not exist"));
    assert!(ok(&mut admin, "users").contains("*.places=read, travel.archive=read"));

    // A project is removed once it is empty.
    assert!(fails(&mut admin, "project remove school").contains("still holds school.places"));
    ok(&mut admin, "workspace remove school.places");
    ok(&mut admin, "project remove school");
    assert!(!ok(&mut admin, "project list").contains("school"));
    let audit = ok(&mut admin, "audit last 100");
    assert!(audit.contains("project create travel") && audit.contains("workspace move old_places to travel.archive"));
}

/// Leaves a home as `workspace move old to travel.old` would leave it if the
/// process stopped after `steps` of its steps: 1, the move recorded and the
/// grants given on the new name too (on the disk); 2, kiva.conf renamed; 3,
/// everything done but the record.
fn move_stopped_after(dir: &TempDir, steps: u8) {
    let inst = instance(dir);
    let mut users = kivadb::auth::list_users(&inst.system).unwrap();
    users.retain(|u| u.grants.contains_key("old"));
    let names: Vec<String> = users.iter().map(|u| u.name.clone()).collect();
    let record = serde_json::json!({ "from": "old", "to": "travel.old", "doubled": names });
    inst.system.put("moving:old", &kivadb::value::Value::Doc(record), 0, false).unwrap();
    for u in &mut users {
        let p = u.grants["old"];
        u.grants.insert("travel.old".into(), p);
        kivadb::auth::save_user(&inst.system, u).unwrap();
    }
    if steps >= 2 {
        inst.cfg_mut().rename("workspaces", "old", "travel.old").unwrap();
    }
    if steps >= 3 {
        for mut u in kivadb::auth::list_users(&inst.system).unwrap() {
            u.grants.remove("old");
            if u.default_workspace.as_deref() == Some("old") {
                u.default_workspace = Some("travel.old".into());
            }
            kivadb::auth::save_user(&inst.system, &u).unwrap();
        }
    }
    inst.shutdown();
}

#[test]
fn a_workspace_move_stopped_at_any_step_is_finished_or_taken_back_at_the_next_start() {
    for steps in 1..=3u8 {
        let dir = TempDir::new(&format!("move_stopped_{steps}"));
        {
            let inst = instance(&dir);
            let mut admin = Session::new(inst.clone(), true, None);
            ok(&mut admin, "project create travel");
            ok(&mut admin, "workspace create old");
            ok(&mut admin, "workspace switch old");
            ok(&mut admin, "set kept 1");
            ok(&mut admin, r#"create user "app" identified by "pw" grant read on old default workspace old"#);
            ok(&mut admin, r#"create user "other" identified by "pw" grant write on *.places"#);
            drop(admin);
            inst.shutdown();
        }
        move_stopped_after(&dir, steps);

        let inst = instance(&dir);
        let mut admin = Session::new(inst.clone(), true, None);
        let users = ok(&mut admin, "users");
        let list = ok(&mut admin, "workspace list");
        let mut app = Session::new(inst.clone(), false, None);
        ok(&mut app, "auth app pw");
        if steps == 1 {
            // kiva.conf still says `old`: the move is taken back, with no
            // grant left on a name that is not a workspace.
            assert!(users.contains("old=read (default: old)") && !users.contains("travel.old"), "{steps}: {users}");
            assert!(list.contains(" old ") && !list.contains("travel.old"), "{steps}: {list}");
            assert_eq!(ok(&mut app, "whoami"), "app (editor) @ workspace 'old'");
        } else {
            // kiva.conf says `travel.old`: the move is finished, the old name
            // is gone from the accounts.
            assert!(users.contains("travel.old=read (default: travel.old)"), "{steps}: {users}");
            assert!(!users.contains(" old=") && !users.contains("|old="), "{steps}: {users}");
            assert!(list.contains("travel.old") && !list.contains(" old "), "{steps}: {list}");
            assert_eq!(ok(&mut app, "whoami"), "app (editor) @ workspace 'travel.old'");
        }
        assert_eq!(ok(&mut app, "get kept"), "kept: 1 [number]");
        // Nobody else gained anything, and the record is gone.
        assert!(users.contains("*.places=write"), "{steps}: {users}");
        assert!(!users.lines().any(|l| l.contains("other") && l.contains("old")), "{steps}: {users}");
        assert!(inst.system.get("moving:old").unwrap().is_none(), "{steps}");
    }
}

#[test]
fn accounts_from_before_2_0_keep_their_personal_workspace() {
    let dir = TempDir::new("model_migration");
    {
        let inst = instance(&dir);
        let mut admin = Session::new(inst.clone(), true, None);
        ok(&mut admin, r#"create user "old" identified by "pw""#);
        // As 1.x left it: a workspace u_old, reached without a grant, and no model in the account.
        inst.create_workspace("u_old", None).unwrap();
        ok(&mut admin, "workspace switch u_old");
        ok(&mut admin, "set mine 1");
        let (value, _) = inst.system.get("user:old").unwrap().unwrap();
        let kivadb::value::Value::Doc(mut doc) = value else { panic!("an account is a document") };
        doc.as_object_mut().unwrap().remove("model");
        doc.as_object_mut().unwrap().remove("default_workspace");
        inst.system.put("user:old", &kivadb::value::Value::Doc(doc), 0, false).unwrap();
        drop(admin);
        inst.shutdown();
    }
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), false, None);
    ok(&mut s, "auth old pw");
    assert_eq!(ok(&mut s, "whoami"), "old (editor) @ workspace 'u_old'");
    assert_eq!(ok(&mut s, "get mine"), "mine: 1 [number]");
    ok(&mut s, "set more 2");
    let mut admin = Session::new(inst, true, None);
    assert!(ok(&mut admin, "users").contains("u_old=write (default: u_old)"));
}

#[test]
fn dates_times_and_datetimes() {
    let dir = TempDir::new("dates");
    let mut s = kiva(&dir);
    ok(&mut s, "config set time.zone Africa/Kinshasa");

    // Typed values: a datetime is an instant, shown in the zone; dates and times have none.
    ok(&mut s, r#"set datetime created_at "2026-10-27 14:30:00""#);
    ok(&mut s, "set date born '2004-05-09'");
    ok(&mut s, r#"set time opens "08:00""#);
    assert_eq!(
        ok(&mut s, "get created_at and born and opens"),
        "created_at: 2026-10-27 14:30:00 +01:00 [datetime]\nborn: 2004-05-09 [date]\nopens: 08:00:00 [time]"
    );
    ok(&mut s, "timezone America/New_York");
    assert_eq!(ok(&mut s, "get created_at"), "created_at: 2026-10-27 09:30:00 -04:00 [datetime]");
    assert_eq!(ok(&mut s, "get born"), "born: 2004-05-09 [date]");
    ok(&mut s, "timezone default");
    // A text with its own zone is that moment, wherever it is read.
    ok(&mut s, r#"update datetime created_at "2026-10-27T13:30:00Z""#);
    assert_eq!(ok(&mut s, "get created_at"), "created_at: 2026-10-27 14:30:00 +01:00 [datetime]");
    assert!(fails(&mut s, r#"set datetime bad "27/10/2026""#).contains("is not a datetime"));
    assert!(fails(&mut s, "set date bad 2026-10-27").contains("A date is quoted"));
    assert!(fails(&mut s, r#"update created_at "2026-10-27""#).contains("Cannot update [datetime]"));
    // Without a type keyword, a date-like text stays text; `date` and `time` remain key names.
    ok(&mut s, r#"set ref "2026-10-27""#);
    assert_eq!(ok(&mut s, "typeof ref"), " -> ref is a [string]");
    ok(&mut s, r#"set date "x""#);
    ok(&mut s, "set time now()");
    assert_eq!(ok(&mut s, "typeof date and typeof time"), " -> date is a [string]\n -> time is a [datetime]");

    // The moment of the action, moved by durations.
    ok(&mut s, "set at now()");
    ok(&mut s, "set due today() + 30 days");
    ok(&mut s, "set soon now() + 2 hours - 30 minutes");
    ok(&mut s, "set lunch time '12:00' + 90 minutes");
    assert_eq!(ok(&mut s, "get lunch"), "lunch: 13:30:00 [time]");
    ok(&mut s, "set at2 current_timestamp");
    assert!(ok(&mut s, "typeof due and typeof at2").contains("due is a [date]"));
    assert!(fails(&mut s, "set late today() + 2 hours").contains("a date moves by days or weeks"));
    assert!(fails(&mut s, "set late now() + 1 month").contains("months and years vary in length"));
    assert!(fails(&mut s, "set number n now()").contains("Value is a datetime, not a number"));

    // Functions, in print and in the session's zone.
    assert_eq!(ok(&mut s, "print year(created_at), month(created_at), day(created_at)"), "2026 10 27");
    assert_eq!(ok(&mut s, "print hour(created_at), weekday(born), minute(opens)"), "14 7 0");
    assert!(fails(&mut s, "print month(42)").contains("takes a date, a time or a datetime"));

    // Documents keep their dates as ISO text, compared as moments.
    ok(&mut s, "set doc {}");
    ok(&mut s, "json set doc .seen now()");
    assert!(ok(&mut s, "get doc.seen").contains("Z"));
    for (k, at) in [
        ("o:1", "2026-10-27T13:30:00Z"),
        ("o:2", "2026-12-31T23:30:00Z"),
        ("o:3", "2025-03-01 10:00"),
        ("o:4", "2026-10-27"),
    ] {
        ok(&mut s, &format!(r#"set {k} {{"at": "{at}", "opens": "08:00"}}"#));
    }
    let keys = |s: &mut Session, q: &str| {
        let out = ok(s, q);
        out.lines()
            .filter_map(|l| l.split(':').nth(1).map(|n| n.to_string()))
            .filter(|n| n.len() == 1)
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_eq!(keys(&mut s, r#"get where o:*.at >= datetime "2026-10-27 14:30" return opens"#), "1,2");
    // 23:30 UTC on 31 December is already 2027 in Kinshasa.
    assert_eq!(keys(&mut s, "get where year(o:*.at) = 2027 return opens"), "2");
    assert_eq!(keys(&mut s, "get where month(o:*.at) = 10 return opens"), "1,4");
    assert_eq!(keys(&mut s, r#"get where o:*.opens >= time "07:30" return opens"#), "1,2,3,4");
    assert_eq!(keys(&mut s, "get where o:*.at < now() - 365 days return opens"), "3");
    assert_eq!(
        ok(&mut s, "count o:* group by year(o:*.at)"),
        "Count by year(at) (3 groups, 4 matches):\n  2025: 1\n  2026: 2\n  2027: 1"
    );
    // An index answers moments and year(): the same documents, fewer read.
    ok(&mut s, "create index on o: (at)");
    let plan = ok(&mut s, "explain get where year(o:*.at) = 2026");
    assert!(plan.contains(r#"at >= datetime "2026-01-01 00:00:00 +01:00""#) && plan.contains("INDEX"), "{plan}");
    assert_eq!(keys(&mut s, "get where year(o:*.at) = 2026 return opens"), "1,4");
    assert!(ok(&mut s, r#"explain get where o:*.at < datetime "2026-01-01""#).contains("INDEX SCAN"));
    assert_eq!(keys(&mut s, r#"get where o:*.at < datetime "2026-01-01" return opens"#), "3");
    assert_eq!(keys(&mut s, r#"get where o:*.at = datetime "2026-10-27 14:30" return opens"#), "1");

    // The server's zone, the session's, and an account's.
    assert!(fails(&mut s, "config set time.zone Mars/Olympus").contains("is not a time zone"));
    assert!(ok(&mut s, "timezone").starts_with("Time zone: Africa/Kinshasa (from the server's time.zone)"));
    assert!(fails(&mut s, "timezone Nowhere").contains("is not a time zone"));
    ok(&mut s, r#"create user "ny" identified by "pw""#);
    ok(&mut s, r#"grant read on default to "ny""#);
    ok(&mut s, r#"modify user "ny" timezone America/New_York"#);
    let mut ny = Session::new(s.inst.clone(), false, None);
    ok(&mut ny, "auth ny pw");
    assert_eq!(ok(&mut ny, "get created_at"), "created_at: 2026-10-27 09:30:00 -04:00 [datetime]");
    assert!(ok(&mut s, "stats").contains("> Clock (UTC)          : "));

    // Programs get ISO 8601 in UTC, whatever the zone, and give a text or milliseconds.
    use kivadb::api::handle;
    use serde_json::json;
    let got = handle(&mut s, &json!({"op": "get", "key": "created_at"})).unwrap();
    assert_eq!((got["type"].as_str(), got["value"].as_str()), (Some("datetime"), Some("2026-10-27T13:30:00.000Z")));
    handle(&mut s, &json!({"op": "put", "key": "api_at", "type": "datetime", "value": "2026-10-27 14:30"})).unwrap();
    handle(&mut s, &json!({"op": "put", "key": "api_ms", "type": "datetime", "value": 1793107800000i64})).unwrap();
    assert_eq!(
        ok(&mut s, "get api_at and api_ms"),
        "api_at: 2026-10-27 14:30:00 +01:00 [datetime]\napi_ms: 2026-10-27 14:30:00 +01:00 [datetime]"
    );
    assert!(handle(&mut s, &json!({"op": "put", "key": "x", "type": "date", "value": "2026-13-01"})).is_err());
    let q = json!({"op": "query", "target": "o:*", "where": {"field": "month(at)", "cmp": "=", "value": 10}});
    assert_eq!(handle(&mut s, &q).unwrap()["rows"].as_array().map(|r| r.len()), Some(2));
    let q = json!({"op": "query", "target": "o:*", "where": {"field": "at", "cmp": "<", "value": {"$datetime": "2026-01-01T00:00:00Z"}}});
    assert_eq!(handle(&mut s, &q).unwrap()["rows"].as_array().map(|r| r.len()), Some(1));
    let agg = json!({"op": "aggregate", "target": "o:*", "functions": ["count"], "group_by": "year(at)"});
    assert_eq!(handle(&mut s, &agg).unwrap()["groups"].as_array().map(|g| g.len()), Some(3));
}

#[test]
fn skip_auth_bypasses_login() {
    let dir = TempDir::new("skipauth");
    {
        let inst = instance(&dir);
        let mut s = Session::new(inst.clone(), true, None);
        ok(&mut s, "config set auth_required true");
        ok(&mut s, r#"set k "v""#);
        drop(s);
        inst.shutdown();
    }
    let inst = Instance::open(dir.path(), Options { skip_auth: true, ..Default::default() }).unwrap();
    let mut s = Session::new(inst, false, None);
    assert_eq!(ok(&mut s, "get k"), "k: v [string]");
}

#[test]
fn workspaces_and_configuration() {
    let dir = TempDir::new("ws");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set loc "default""#);
    let custom = dir.path().join("elsewhere").display().to_string();
    assert!(ok(&mut s, "workspace create projet_a").contains("created"));
    ok(&mut s, &format!("workspace create projet_b \"{custom}\""));
    assert!(fails(&mut s, "workspace create projet_a").contains("already exists"));
    assert!(fails(&mut s, "workspace create \"bad name\"").contains("may only contain"));
    assert!(fails(&mut s, "workspace switch nope").contains("does not exist"));

    assert!(ok(&mut s, "workspace switch projet_a").contains("(0 keys)"));
    ok(&mut s, r#"set loc "a""#);
    ok(&mut s, "workspace switch projet_b");
    ok(&mut s, r#"set loc "b""#);
    assert!(std::path::Path::new(&custom).join("store.kiva").exists());
    ok(&mut s, "workspace switch default");
    assert_eq!(ok(&mut s, "get loc"), "loc: default [string]");
    assert!(ok(&mut s, "workspace list").contains(" * default"));
    assert!(ok(&mut s, "workspace current").starts_with("default ("));
    assert!(fails(&mut s, "workspace remove default").contains("cannot be removed"));

    assert_eq!(ok(&mut s, "config get auto_compact"), "storage.auto_compact = true");
    ok(&mut s, "config set compact_threshold 25");
    // Settings read at every use say nothing about a restart; the others do.
    for live in [
        "semantic.cache.enabled true",
        "failover.auto true",
        "tls.required false",
        "http.stream true",
        "server.max_clients 500",
    ] {
        let out = ok(&mut s, &format!("config set {live}"));
        assert!(!out.contains("next time KivaDB starts"), "{live}: {out}");
    }
    assert!(ok(&mut s, "config set storage.snapshot_interval 300").contains("next time KivaDB starts"));
    assert_eq!(s.engine.auto_compact(), (true, 25));
    assert!(ok(&mut s, "autocompact off").contains("OFF"));
    assert_eq!(s.engine.auto_compact(), (false, 25));
    assert!(ok(&mut s, "autocompact threshold 40").contains("threshold 40%"));
    assert!(ok(&mut s, "autocompact on").contains("ON"));
    assert!(ok(&mut s, "autocompact status").contains("Threshold: 40%"));
    assert!(fails(&mut s, "autocompact threshold 150").contains("between 1 and 99"));
    assert!(fails(&mut s, "config set compact_threshold abc").contains("invalid value"));
    assert!(fails(&mut s, "config get nonsense").contains("unknown configuration key"));
    assert!(fails(&mut s, "config get port").contains("network.port, http.port, resp.port"));
    assert_eq!(ok(&mut s, "config get network.port"), "network.port = 2911");
    ok(&mut s, "config set master_password hunter2");
    let list = ok(&mut s, "config list");
    assert!(list.contains("replication.master_password") && !list.contains("hunter2"));
    assert!(list.contains("workspaces.projet_a"));
    drop(s);
    inst.shutdown();
    drop(inst);

    // Configuration and workspaces persist.
    let mut s = kiva(&dir);
    assert_eq!(ok(&mut s, "config get compact_threshold"), "storage.compact_threshold = 40");
    ok(&mut s, "workspace switch projet_b");
    assert_eq!(ok(&mut s, "get loc"), "loc: b [string]");
}

#[test]
fn maintenance_commands() {
    let dir = TempDir::new("maint");
    let mut s = kiva(&dir);
    let text = "compressible ".repeat(50);
    for i in 0..30 {
        ok(&mut s, &format!("set k{i:02} \"{text}\""));
        ok(&mut s, &format!("update k{i:02} \"{text}!\""));
    }
    let stats = ok(&mut s, "stats");
    assert!(stats.contains("Total Keys           : 30") && stats.contains("[Action Needed: 'compact']"));
    assert!(ok(&mut s, "compact").contains("30 records"));
    assert!(ok(&mut s, "stats").contains("Disk Fragmentation   : 0.0%"));
    assert!(fails(&mut s, "compact now").contains("does not accept arguments"));

    let before = s.engine.stats().file_bytes;
    assert!(ok(&mut s, "compress lz4").contains("compression 'lz4'"));
    assert!(s.engine.stats().file_bytes < before / 3);
    assert!(ok(&mut s, "stats").contains("Compression          : lz4"));
    assert_eq!(ok(&mut s, "config get compression"), "storage.compression = lz4");
    assert_eq!(ok(&mut s, "get k07"), format!("k07: {text}! [string]"));
    ok(&mut s, "compress zlib");
    assert!(ok(&mut s, "scan").contains("zlib"));
    ok(&mut s, "compress none");
    assert_eq!(s.engine.stats().compressed_records, 0);
    assert!(fails(&mut s, "compress brotli").contains("Usage"));

    let check = ok(&mut s, "check");
    assert!(check.contains("Checking 30 entries...") && check.contains("OK: 30") && check.contains("CORRUPTED: 0"));
    let hint = ok(&mut s, "hint info");
    assert!(hint.contains("Status: VALID") && hint.contains("Entries: 30") && hint.contains("Checksum: PASSED"));
    assert!(ok(&mut s, "bench write 200").starts_with("WRITE: 200 ops"));
    assert!(ok(&mut s, "bench read 200").starts_with("READ : 200 ops"));
    assert_eq!(s.engine.len(), 30);
    assert_eq!(ok(&mut s, "bogus command"), "Unknown command: 'bogus'. Try 'help'.");
    assert!(ok(&mut s, "help").contains("TRANSACTIONS"));
    assert_eq!(ok(&mut s, "ping"), "PONG");
    assert_eq!(ok(&mut s, "version"), concat!("KivaDB ", env!("CARGO_PKG_VERSION")));
}

#[test]
fn lua_scripts_and_triggers() {
    let dir = TempDir::new("lua");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, r#"set user:1 {"name": "Alice", "tags": ["a", "b"]} and n 5 and txt "hi""#);
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("user:1").name'"#), "Alice");
    assert_eq!(ok(&mut s, r#"eval 'return kiva.get("n") + 1, kiva.get("txt"), kiva.get("ghost")'"#), "6, hi, (nil)");
    assert_eq!(
        ok(&mut s, r#"eval 'return #kiva.get("user:1").tags, kiva.typeof("user:1"), kiva.has("n")'"#),
        "2, object, true"
    );
    assert_eq!(ok(&mut s, r#"eval 'print("hello", ARGV[1], ARGV[2])' world 42"#), "hello\tworld\t42\nOK");
    ok(&mut s, r#"eval 'kiva.set("made", {list = {1, 2, 3}, ok = true}); kiva.set("num", 1.5); kiva.del("txt")'"#);
    assert_eq!(
        ok(&mut s, "get made.list and num and txt"),
        "made.list: [1,2,3] [array]\nnum: 1.5 [number]\ntxt: (nil)"
    );
    assert_eq!(ok(&mut s, r#"eval 'return kiva.incr("n", 10)'"#), "15");
    assert_eq!(ok(&mut s, r#"eval 'return #kiva.scan("user:*")'"#), "1");

    // Atomic: an error undoes everything the script wrote.
    assert!(fails(&mut s, r#"eval 'kiva.set("half", 1); kiva.del("n"); error("boom")'"#).contains("boom"));
    assert_eq!(ok(&mut s, "get half and n"), "half: (nil)\nn: 15 [number]");
    assert!(fails(&mut s, "eval 'this is not lua'").contains("ScriptError"));
    assert!(fails(&mut s, r#"eval 'kiva.set("123", 1)'"#).contains("purely numeric"));

    // Sandbox: no file system, no OS, no code loading; runaway scripts are stopped.
    for code in [
        "return io.open('x')",
        "return os.execute('dir')",
        "return require('os')",
        "return load('return 1')()",
        "return dofile('x')",
    ] {
        assert!(fails(&mut s, &format!("eval \"{code}\"")).contains("ScriptError"), "{code} should be blocked");
    }
    ok(&mut s, "config set timeout_ms 300");
    let start = Instant::now();
    assert!(fails(&mut s, "eval 'while true do end'").contains("time limit"));
    assert!(start.elapsed() < Duration::from_secs(5));
    ok(&mut s, "config set timeout_ms 5000");

    // Stored scripts.
    let file = dir.path().join("increment.lua");
    std::fs::write(&file, "local key, by = ARGV[1], tonumber(ARGV[2] or '1')\nreturn kiva.incr(key, by)\n").unwrap();
    assert!(ok(&mut s, &format!("script load \"{}\" as \"inc\"", file.display())).contains("Script 'inc' stored"));
    assert_eq!(ok(&mut s, r#"script run "inc" "n" 5"#), "20");
    assert_eq!(ok(&mut s, r#"script run "inc" fresh"#), "1");
    assert!(ok(&mut s, "script list").starts_with("inc"));
    assert!(ok(&mut s, r#"script show "inc""#).contains("kiva.incr"));
    assert!(fails(&mut s, r#"script run "nope""#).contains("does not exist"));
    let bad = dir.path().join("bad.lua");
    std::fs::write(&bad, "return (((").unwrap();
    assert!(fails(&mut s, &format!("script load \"{}\" as \"bad\"", bad.display())).contains("syntax error"));
    assert!(ok(&mut s, r#"bench script "inc" iterations 50 counter"#).starts_with("SCRIPT inc: 50 ops"));
    assert_eq!(ok(&mut s, "get counter"), "counter: 50 [number]");

    // Triggers run the script with ARGV = {event, key}.
    let audit = dir.path().join("audit_log.lua");
    std::fs::write(&audit, "kiva.set('log:' .. ARGV[1] .. ':' .. ARGV[2], 'seen')\n").unwrap();
    ok(&mut s, &format!("script load \"{}\" as \"audit_log\"", audit.display()));
    assert!(fails(&mut s, r#"create trigger on set "user:*" run "missing""#).contains("does not exist"));
    ok(&mut s, r#"create trigger watch_set on set "user:*" run "audit_log.lua""#);
    ok(&mut s, r#"create trigger watch_del on del "user:*" run "audit_log""#);
    assert!(ok(&mut s, "triggers").contains("watch_set"));
    ok(&mut s, r#"set user:9 "new" and other "ignored""#);
    ok(&mut s, "del user:9");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !(s.engine.contains("log:set:user:9") && s.engine.contains("log:del:user:9")) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(s.engine.contains("log:set:user:9") && s.engine.contains("log:del:user:9"));
    assert!(!s.engine.contains("log:set:other"));
    ok(&mut s, "drop trigger watch_set");
    ok(&mut s, r#"script delete "inc""#);
    assert!(fails(&mut s, r#"script delete "inc""#).contains("does not exist"));
}

#[test]
fn subscriptions_announce_changes() {
    let dir = TempDir::new("pubsub");
    let inst = instance(&dir);
    let (tx, rx) = mpsc::channel();
    let mut watcher = Session::new(inst.clone(), true, Some(tx));
    let mut writer = Session::new(inst.clone(), true, None);
    assert!(fails(&mut writer, "subscribe user:*").contains("cannot receive notifications"));
    ok(&mut watcher, "subscribe user:*");
    ok(&mut writer, r#"set user:1 "a" and order:1 "ignored""#);
    ok(&mut writer, r#"update user:1 "b""#);
    ok(&mut writer, "del user:1");
    let got: Vec<String> = (0..3).map(|_| rx.recv_timeout(Duration::from_secs(2)).unwrap()).collect();
    assert_eq!(got, ["set user:1 (string)", "update user:1 (string)", "del user:1"]);
    assert!(rx.try_recv().is_err());
    assert!(ok(&mut watcher, "unsubscribe").contains("1 subscription(s) removed"));
    ok(&mut writer, r#"set user:2 "x""#);
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn the_instance_is_exclusive() {
    let dir = TempDir::new("exclusive");
    let inst = instance(&dir);
    assert!(matches!(Instance::open(dir.path(), Options::default()), Err(kivadb::KivaError::Locked(_))));
    let _ = run(&mut Session::new(inst.clone(), true, None), "set a 1");
}

#[test]
fn json_documents_from_redis_clients_are_kivadb_documents() {
    use kivadb::resp::{Conn, Reply};
    let dir = TempDir::new("resp-json");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "create index on user:*.age");
    let mut c = Conn::new(Session::new(inst, true, None));
    let call = |c: &mut Conn, args: &[&str]| c.handle(&args.iter().map(|a| a.as_bytes().to_vec()).collect::<Vec<_>>());
    let bulk = |t: &str| Reply::Bulk(Some(t.as_bytes().to_vec()));
    let ok_reply = Reply::Simple("OK".into());

    let user = r#"{"name":"Amani","age":30,"tags":["a"],"embedding":[1,0,0]}"#;
    assert_eq!(call(&mut c, &["JSON.SET", "user:1", "$", user]), ok_reply);
    assert_eq!(
        call(&mut c, &["JSON.SET", "user:2", ".", r#"{"name":"Baraka","age":25,"embedding":[0,1,0]}"#]),
        ok_reply
    );
    assert_eq!(call(&mut c, &["JSON.GET", "user:1", "$.name"]), bulk(r#"["Amani"]"#));
    assert_eq!(call(&mut c, &["JSON.GET", "user:1", ".name"]), bulk(r#""Amani""#));
    assert_eq!(call(&mut c, &["JSON.GET", "user:1", "$.nothing"]), bulk("[]"));
    assert_eq!(call(&mut c, &["JSON.GET", "ghost"]), Reply::Bulk(None));
    assert_eq!(
        call(&mut c, &["JSON.GET", "user:2", "INDENT", "  ", ".name", ".age"]),
        bulk(r#"{".name":"Baraka",".age":25}"#)
    );

    // Changes in place, at a path.
    assert_eq!(call(&mut c, &["JSON.SET", "user:1", "$.address.city", r#""Goma""#]), ok_reply);
    assert_eq!(call(&mut c, &["JSON.SET", "user:1", "$.name", r#""X""#, "NX"]), Reply::Bulk(None));
    assert_eq!(call(&mut c, &["JSON.SET", "user:1", "$.zip", "1", "XX"]), Reply::Bulk(None));
    assert_eq!(call(&mut c, &["JSON.NUMINCRBY", "user:1", "$.age", "1"]), bulk("[31]"));
    assert_eq!(call(&mut c, &["JSON.NUMINCRBY", "user:1", ".age", "1"]), bulk("32"));
    assert_eq!(call(&mut c, &["JSON.NUMMULTBY", "user:2", ".age", "2"]), bulk("50"));
    assert!(
        matches!(call(&mut c, &["JSON.NUMINCRBY", "user:1", ".nothing", "1"]), Reply::Error(e) if e.contains("does not exist"))
    );
    assert_eq!(
        call(&mut c, &["JSON.ARRAPPEND", "user:1", "$.tags", r#""b""#, r#""c""#]),
        Reply::Array(Some(vec![Reply::Int(3)]))
    );
    assert_eq!(call(&mut c, &["JSON.ARRINSERT", "user:1", ".tags", "0", r#""z""#]), Reply::Int(4));
    assert_eq!(call(&mut c, &["JSON.ARRLEN", "user:1", ".tags"]), Reply::Int(4));
    assert_eq!(call(&mut c, &["JSON.STRLEN", "user:1", ".name"]), Reply::Int(5));
    assert_eq!(call(&mut c, &["JSON.TYPE", "user:1", ".tags"]), Reply::Simple("array".into()));
    assert_eq!(
        call(&mut c, &["JSON.TYPE", "user:1", "$.age"]),
        Reply::Array(Some(vec![Reply::Simple("integer".into())]))
    );
    assert_eq!(call(&mut c, &["JSON.OBJLEN", "user:1", "$.address"]), Reply::Array(Some(vec![Reply::Int(1)])));
    assert_eq!(call(&mut c, &["JSON.DEL", "user:1", "$.address.city"]), Reply::Int(1));
    assert_eq!(call(&mut c, &["JSON.DEL", "user:1", "$.address.city"]), Reply::Int(0));
    assert_eq!(
        call(&mut c, &["JSON.MGET", "user:1", "ghost", "user:2", "$.name"]),
        Reply::Array(Some(vec![bulk(r#"["Amani"]"#), Reply::Bulk(None), bulk(r#"["Baraka"]"#)]))
    );
    assert!(matches!(call(&mut c, &["JSON.GET", "user:1", "$..name"]), Reply::Error(e) if e.contains("wildcards")));
    assert!(matches!(call(&mut c, &["JSON.SET", "new", "$.a", "1"]), Reply::Error(e) if e.contains("at the root")));
    assert!(matches!(call(&mut c, &["JSON.SET", "user:1", "$", "{oops"]), Reply::Error(e) if e.contains("JSON")));

    // The same data, through every model: hashes, queries, indexes, vectors.
    assert_eq!(call(&mut c, &["HGET", "user:1", "name"]), bulk("Amani"));
    assert_eq!(call(&mut c, &["TYPE", "user:1"]), Reply::Simple("hash".into()));
    let found = ok(&mut s, "get where user:*.age < 40");
    assert!(
        found.contains("user:1") && found.contains(r#""tags":["z","a","b","c"]"#) && !found.contains("user:2"),
        "{found}"
    );
    assert!(ok(&mut s, "explain get where user:*.age < 40").contains("INDEX SCAN"));
    let Reply::Array(Some(rows)) = call(&mut c, &["KIVA.SIMILAR", "user:*.embedding", "[0.9, 0.1, 0]", "LIMIT", "1"])
    else {
        panic!("not an array");
    };
    let Reply::Array(Some(first)) = &rows[0] else { panic!() };
    assert_eq!(first[0], bulk("user:1"));
    // A failing script undoes its JSON changes with the rest.
    let script = "redis.call('JSON.NUMINCRBY', KEYS[1], '.age', 100) redis.call('JSON.DEL', KEYS[1]) error('boom')";
    assert!(matches!(call(&mut c, &["EVAL", script, "1", "user:1"]), Reply::Error(e) if e.contains("boom")));
    assert_eq!(call(&mut c, &["JSON.GET", "user:1", ".age"]), bulk("32"));
    // Written from the shell, read with JSON.GET.
    ok(&mut s, r#"set conf:app {"theme": "dark", "limits": {"rpm": 600}}"#);
    assert_eq!(call(&mut c, &["JSON.GET", "conf:app", "$.limits.rpm"]), bulk("[600]"));
    // A Redis string reads as a JSON string.
    assert_eq!(call(&mut c, &["SET", "greeting", "hi"]), ok_reply);
    assert_eq!(call(&mut c, &["JSON.GET", "greeting"]), bulk(r#""hi""#));
    assert_eq!(call(&mut c, &["JSON.DEL", "user:2"]), Reply::Int(1));
    assert_eq!(call(&mut c, &["EXISTS", "user:2"]), Reply::Int(0));
}

#[test]
fn queries_combine_conditions_and_aggregate_per_group() {
    let dir = TempDir::new("query-logic");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set user:1 {"name": "Alice", "role": "admin", "age": 30, "city": "Kinshasa", "salary": 900}"#);
    ok(&mut s, r#"set user:2 {"name": "Bob", "role": "dev", "age": 25, "city": "Goma", "salary": 700}"#);
    ok(&mut s, r#"set user:3 {"name": "Carol", "role": "dev", "age": 41, "city": "Kinshasa", "salary": 1100}"#);
    ok(&mut s, r#"set user:4 {"name": "Dan", "role": "ops", "age": 35, "salary": "n/a"}"#);
    ok(&mut s, r#"set order:1 {"user": "user:1", "total": 10}"#);
    let keys = |out: String| -> Vec<String> {
        out.lines()
            .filter(|l| l.starts_with("user:"))
            .map(|l| l.split(':').take(2).collect::<Vec<_>>().join(":"))
            .collect()
    };

    // and, or, not, parentheses; the pattern is named once.
    let q = r#"get where user:*.age > 26 and (role = "dev" or role = "admin")"#;
    assert_eq!(keys(ok(&mut s, q)), ["user:1", "user:3"]);
    assert_eq!(
        keys(ok(&mut s, r#"get where user:*.role = "dev" or user:*.city = "Kinshasa""#)),
        ["user:1", "user:2", "user:3"]
    );
    // not: also the documents without the field.
    assert_eq!(keys(ok(&mut s, r#"get where not user:*.city = "Kinshasa""#)), ["user:2", "user:4"]);
    assert_eq!(
        keys(ok(&mut s, r#"get where user:*.age >= 30 and not (city = "Goma" or role = "ops")"#)),
        ["user:1", "user:3"]
    );
    // and binds tighter than or.
    assert_eq!(keys(ok(&mut s, r#"get where user:*.role = "ops" or role = "dev" and age > 30"#)), ["user:3", "user:4"]);
    assert_eq!(
        ok(&mut s, r#"get where user:*.role = "dev" and age < 30 return name"#),
        "user:2: {\"name\":\"Bob\"}\n(1 matches)"
    );

    // The existing aggregates keep their output.
    assert_eq!(ok(&mut s, "count where user:*.age > 26"), "Count: 3");
    assert_eq!(ok(&mut s, "sum where user:*.age > 0"), "Sum: 131 (4 matches)");
    assert_eq!(ok(&mut s, r#"count where user:*.role = "dev" and city = "Goma""#), "Count: 1");
    // New ones, on a field other than the condition's.
    assert_eq!(ok(&mut s, "avg where user:*.age > 0"), "Avg: 32.75 (4 values, 4 matches)");
    assert_eq!(ok(&mut s, r#"avg user:*.salary where role = "dev""#), "Avg: 900 (2 values, 2 matches)");
    assert_eq!(ok(&mut s, "avg user:*.salary"), "Avg: 900 (3 values, 4 matches)", "\"n/a\" is not a number");
    assert_eq!(ok(&mut s, "min user:*.age"), "Min: 25 (4 values, 4 matches)");
    assert_eq!(ok(&mut s, r#"max user:*.name where not role = "ops""#), "Max: \"Carol\" (3 values, 3 matches)");
    assert_eq!(ok(&mut s, "count user:*"), "Count: 4");
    assert_eq!(ok(&mut s, "count user:*.city"), "Count: 3 (4 matches)");
    // Per group: missing values form the null group, first.
    assert_eq!(
        ok(&mut s, "count user:* group by city"),
        "Count by city (3 groups, 4 matches):\n  null: 1\n  \"Goma\": 1\n  \"Kinshasa\": 2"
    );
    assert_eq!(
        ok(&mut s, r#"avg user:*.age where role != "ops" group by role"#),
        "Avg of age by role (2 groups, 3 matches):\n  \"admin\": 30 (1 values)\n  \"dev\": 33 (2 values)"
    );
    assert_eq!(
        ok(&mut s, "sum user:*.salary group by user:*.city limit 3"),
        "Sum of salary by city (2 groups, 3 matches):\n  \"Goma\": 700 (1 matches)\n  \"Kinshasa\": 2000 (2 matches)"
    );

    // Plans: indexes intersected for and, merged for or, none through not.
    ok(&mut s, "create index on user: (role)");
    ok(&mut s, "create index on user: (age)");
    let plan = ok(&mut s, r#"explain get where user:*.role = "dev" and age > 30"#);
    assert!(plan.contains("INDEX MERGE") && plan.contains("user:* (role) and user:* (age)"), "{plan}");
    assert!(plan.contains("1 of 5 keys examined"), "{plan}");
    let plan = ok(&mut s, r#"explain count where user:*.role = "dev" or role = "ops""#);
    assert!(plan.contains("INDEX MERGE") && plan.contains("3 of 5 keys"), "{plan}");
    let plan = ok(&mut s, r#"explain avg user:*.age where role = "dev" and city = "Goma" group by city"#);
    assert!(plan.contains("INDEX SCAN") && plan.contains("2 of 5 keys"), "{plan}");
    let plan = ok(&mut s, r#"explain get where user:*.role = "dev" or city = "Goma""#);
    assert!(plan.contains("PREFIX SCAN") && plan.contains("Tip       : create index on user:* (city)"), "{plan}");
    assert!(!plan.contains("index on user:* (role)"), "{plan}");
    let plan = ok(&mut s, r#"explain get where not user:*.role = "dev""#);
    assert!(plan.contains("PREFIX SCAN") && !plan.contains("Tip"), "{plan}");
    // Same answers through the indexes.
    assert_eq!(keys(ok(&mut s, q)), ["user:1", "user:3"]);
    assert_eq!(
        ok(&mut s, r#"avg user:*.age where role != "ops" group by role"#),
        "Avg of age by role (2 groups, 3 matches):\n  \"admin\": 30 (1 values)\n  \"dev\": 33 (2 values)"
    );

    // Structured requests.
    let api = |s: &mut Session, r: serde_json::Value| kivadb::api::handle(s, &r).unwrap();
    let r = api(
        &mut s,
        serde_json::json!({"op": "query", "target": "user:*",
            "where": {"and": [{"field": "age", "cmp": ">", "value": 26}, {"or": [{"role": "dev"}, {"role": "admin"}]}]}}),
    );
    let got: Vec<&str> = r["rows"].as_array().unwrap().iter().map(|x| x["key"].as_str().unwrap()).collect();
    assert_eq!(got, ["user:1", "user:3"]);
    assert_eq!(r["plan"]["strategy"], "INDEX MERGE");
    let r = api(
        &mut s,
        serde_json::json!({"op": "aggregate", "target": "user:*", "where": {"not": {"role": "ops"}}, "group_by": "role",
            "compute": {"n": "count", "mean_age": {"avg": "age"}, "top": {"max": "salary"}}}),
    );
    assert_eq!(r["matches"], 3);
    assert_eq!(
        r["groups"][1],
        serde_json::json!({"key": "dev", "matches": 2, "values": {"n": 2, "mean_age": 33, "top": 1100}})
    );
    let r =
        api(&mut s, serde_json::json!({"op": "aggregate", "target": "user:*", "compute": ["count", {"sum": "age"}]}));
    assert_eq!(r["values"], serde_json::json!({"count": 4, "sum_age": 131}));
    let r =
        api(&mut s, serde_json::json!({"op": "explain", "target": "user:*", "where": {"role": "dev", "city": "Goma"}}));
    assert_eq!(r["plan"]["strategy"], "INDEX SCAN");
    let r = api(
        &mut s,
        serde_json::json!({"op": "explain", "target": "user:*", "where": {"or": [{"role": "dev"}, {"city": "Goma"}]}}),
    );
    assert_eq!(r["suggestion"], "create index on user:* (city)");
    for (bad, why) in [
        (serde_json::json!({"op": "query", "target": "user:*.age", "where": {"age": 1}}), "key pattern alone"),
        (serde_json::json!({"op": "aggregate", "target": "user:*.age"}), "key pattern alone"),
        (serde_json::json!({"op": "aggregate", "target": "user:*", "compute": {"m": "median"}}), "compute"),
        (serde_json::json!({"op": "aggregate", "target": "user:*", "order_by": "age"}), "sorting pipelines"),
    ] {
        let err = kivadb::api::handle(&mut s, &bad).unwrap_err().to_string();
        assert!(err.contains(why), "{bad}: {err}");
    }
}

#[test]
fn indexed_and_scanned_queries_give_the_same_answers() {
    let dir = TempDir::new("query-plans");
    let mut s = kiva(&dir);
    let mut seed = 7u64;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    let roles = ["dev", "ops", "admin", "sales"];
    for i in 0..400 {
        let (role, age, city) = (roles[next(4) as usize], 18 + next(50), next(6));
        // Some documents lack a field, some hold another type.
        let doc = match i % 10 {
            0 => format!(r#"{{"role": "{role}", "age": "{age}"}}"#),
            1 => format!(r#"{{"age": {age}, "city": {city}}}"#),
            _ => format!(r#"{{"role": "{role}", "age": {age}, "city": {city}}}"#),
        };
        ok(&mut s, &format!("set p:{i:03} {doc}"));
    }
    ok(&mut s, r#"set other:1 {"role": "dev", "age": 30}"#);
    let queries = [
        r#"p:*.role = "dev" and age > 40"#,
        r#"p:*.role = "dev" or role = "ops""#,
        r#"p:*.age >= 30 and age < 35 and city = 2"#,
        r#"(p:*.role = "admin" or age < 20) and city != 3"#,
        r#"p:*.role = "sales" and not (age > 50 or city = 1)"#,
        r#"p:*.city = 0 or city = 5 or role = "admin" and age <= 25"#,
        r#"not p:*.role = "dev""#,
        r#"p:*.role in ["dev", "ops"] and age > 30"#,
        r#"p:*.city in [0, 4, "4"] or role in ["admin"]"#,
        r#"p:*.role not in ["dev", "sales"]"#,
        r#"p:*.role like "d%" or role like "%ps""#,
        r#"p:*.role ilike "A_M%" and age < 40"#,
        r#"p:*.role not ilike "%E%""#,
    ];
    let run_all = |s: &mut Session| -> Vec<String> {
        let mut out = Vec::new();
        for q in queries {
            out.push(ok(s, &format!("get where {q}")));
            out.push(ok(s, &format!("count where {q} group by role")));
            out.push(ok(s, &format!("avg p:*.age where {} group by city", q.trim_start_matches("p:*."))));
        }
        out
    };
    let scanned = run_all(&mut s);
    ok(&mut s, "create index on p: (role) using hash");
    ok(&mut s, "create index on p: (age)");
    ok(&mut s, "create index on p: (city)");
    let plan = ok(&mut s, &format!("explain get where {}", queries[2]));
    assert!(plan.contains("INDEX MERGE"), "{plan}");
    assert_eq!(run_all(&mut s), scanned);
    assert!(scanned[0].ends_with("matches)") && !scanned[0].starts_with("(0"), "{}", scanned[0]);
}

#[test]
fn in_like_and_ilike_compare_with_lists_and_text_patterns() {
    let dir = TempDir::new("in-like");
    let mut s = kiva(&dir);
    for (i, (name, kind)) in [
        ("Kinshasa", "VILLE"),
        ("Kintambo", "COMMUNE"),
        ("kinkole", "QUARTIER"),
        ("Goma", "VILLE"),
        ("Élisabethville", "VILLE"),
        ("100% Bandal", "QUARTIER"),
    ]
    .iter()
    .enumerate()
    {
        ok(&mut s, &format!(r#"set place:{i} {{"name": "{name}", "type": "{kind}", "parent": {}}}"#, i % 3));
    }
    ok(&mut s, r#"set place:9 {"name": ["not", "text"], "type": null}"#);
    let keys = |s: &mut Session, q: &str| -> Vec<String> {
        let out = ok(s, &format!("get where {q}"));
        out.lines().filter_map(|l| l.split(':').nth(1).map(|n| format!("place:{n}"))).collect()
    };
    let cases: [(&str, &[&str]); 11] = [
        (r#"place:*.type in ["VILLE", "COMMUNE"]"#, &["place:0", "place:1", "place:3", "place:4"]),
        (r#"place:*.parent in [0, 2]"#, &["place:0", "place:2", "place:3", "place:5"]),
        (r#"place:*.type not in ["VILLE"]"#, &["place:1", "place:2", "place:5", "place:9"]),
        (r#"place:*.type in [null]"#, &["place:9"]),
        (r#"place:*.name like "Kin%""#, &["place:0", "place:1"]),
        (r#"place:*.name ilike "kin%""#, &["place:0", "place:1", "place:2"]),
        (r#"place:*.name ilike "GOMA""#, &["place:3"]),
        (r#"place:*.name ilike "é%""#, &["place:4"]),
        (r#"place:*.name like "%a_a""#, &["place:0"]),
        (r#"place:*.name like "100\% %""#, &["place:5"]),
        // A name that is not text fails like, so not like holds.
        (r#"place:*.name not like "%a%" and type != "QUARTIER""#, &["place:9"]),
    ];
    let check = |s: &mut Session| {
        for (q, want) in cases {
            assert_eq!(keys(s, q), want, "{q}");
        }
    };
    check(&mut s);
    ok(&mut s, "create index on place: (type) using hash");
    ok(&mut s, "create index on place: (name)");
    ok(&mut s, "create index on place: (parent)");
    check(&mut s);
    let plan = ok(&mut s, r#"explain get where place:*.type in ["VILLE", "COMMUNE"]"#);
    assert!(plan.contains("INDEX SCAN") && plan.contains("using hash") && plan.contains("4 of 7"), "{plan}");
    let plan = ok(&mut s, r#"explain get where place:*.name like "Kin%""#);
    assert!(plan.contains("place:* (name)") && plan.contains("2 of 7"), "{plan}");
    // ilike tests each distinct value of the index, without reading the documents.
    let plan = ok(&mut s, r#"explain get where place:*.name ilike "kin%""#);
    assert!(plan.contains("INDEX SCAN") && plan.contains("3 of 7"), "{plan}");
    assert!(ok(&mut s, r#"explain get where place:*.type not in ["VILLE"]"#).contains("PREFIX SCAN"));

    for (q, why) in [
        (r#"get where place:*.type in []"#, "at least one value"),
        (r#"get where place:*.type in [["VILLE"]]"#, "strings, numbers, booleans or null"),
        (r#"get where place:*.name like Kin"#, "must be quoted"),
        (r#"get where place:*.name ilike ["a"]"#, "quoted text pattern"),
    ] {
        let err = fails(&mut s, q);
        assert!(err.contains(why), "`{q}`: {err}");
    }
    let many: Vec<String> = (0..=kivadb::engine::MAX_IN_VALUES).map(|i| i.to_string()).collect();
    let err = fails(&mut s, &format!("get where place:*.parent in [{}]", many.join(", ")));
    assert!(err.contains("at most 1024 values"), "{err}");

    // Aggregates, vector filters and structured requests take them too.
    assert_eq!(ok(&mut s, r#"count where place:*.name ilike "%ville""#), "Count: 1");
    ok(&mut s, r#"set doc:1 {"lang": "fr", "v": [1, 0]}"#);
    ok(&mut s, r#"set doc:2 {"lang": "ln", "v": [0.9, 0.1]}"#);
    ok(&mut s, r#"set doc:3 {"lang": "en", "v": [0.8, 0.2]}"#);
    let found = ok(&mut s, r#"get similar doc:*.v to [1, 0] where lang in ["fr", "ln"] limit 5"#);
    assert!(found.contains("doc:1") && found.contains("doc:2") && !found.contains("doc:3"), "{found}");
    assert!(fails(&mut s, r#"get similar doc:*.v to [1, 0] where lang not in ["en"]"#).contains("not in"));

    let api = |s: &mut Session, req: serde_json::Value| kivadb::api::handle(s, &req).unwrap();
    let r =
        api(&mut s, serde_json::json!({"op": "query", "target": "place:*", "where": {"type": ["VILLE", "COMMUNE"]}}));
    assert_eq!(r["rows"].as_array().unwrap().len(), 4);
    let r = api(
        &mut s,
        serde_json::json!({"op": "query", "target": "place:*", "where": {"field": "name", "cmp": "ilike", "value": "KIN%"}}),
    );
    assert_eq!(r["rows"].as_array().unwrap().len(), 3);
    let r = api(&mut s, serde_json::json!({"op": "query", "target": "place:*.name", "cmp": "like", "value": "G%"}));
    assert_eq!(r["rows"][0]["key"], "place:3");
    let r = api(
        &mut s,
        serde_json::json!({"op": "similar", "target": "doc:*.v", "vector": [1, 0], "filter": {"lang": ["en"]}}),
    );
    assert_eq!(r["results"][0]["key"], "doc:3");
}

#[test]
fn unique_indexes_refuse_a_second_document_with_the_same_values() {
    let dir = TempDir::new("unique");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set user:1 {"email": "amani@exemple.cd"}"#);
    ok(&mut s, r#"set user:2 {"email": "amani@exemple.cd"}"#);
    let err = fails(&mut s, "create unique index on user: (email)");
    assert!(err.contains("user:1 and user:2 both hold \"amani@exemple.cd\""), "{err}");
    assert!(ok(&mut s, "indexes").contains("No secondary indexes"), "the refused index is not kept");
    ok(&mut s, r#"update user:2 {"email": "bisimwa@exemple.cd"}"#);
    assert_eq!(
        ok(&mut s, "create unique index on user: (email) using hash"),
        "Unique index created on user:* (email) using hash: 2 documents indexed."
    );
    let err = fails(&mut s, r#"set user:3 {"email": "amani@exemple.cd"}"#);
    assert!(err.contains("'user:3' not written: 'user:1' already holds \"amani@exemple.cd\""), "{err}");
    assert!(err.contains("unique index on user:* (email)"), "{err}");
    assert!(fails(&mut s, r#"update user:2 {"email": "amani@exemple.cd"}"#).contains("already holds"));
    assert!(fails(&mut s, r#"json set user:2 .email "amani@exemple.cd""#).contains("already holds"));
    // Writing a document again with its own value, or without the field, is allowed.
    ok(&mut s, r#"update user:1 {"email": "amani@exemple.cd", "age": 30}"#);
    // A document without the field holds nothing; null is a value like the others.
    ok(&mut s, r#"set user:4 {"name": "no email"} and user:5 {"name": "none either"} and user:6 {"email": null}"#);
    assert!(fails(&mut s, r#"set user:13 {"email": null}"#).contains("already holds null"));
    // Other patterns are not concerned.
    ok(&mut s, r#"set admin:1 {"email": "amani@exemple.cd"}"#);

    // A transaction is judged by what the keys hold at its end.
    ok(&mut s, "begin");
    ok(&mut s, r#"update user:1 {"email": "bisimwa@exemple.cd"}"#);
    ok(&mut s, r#"update user:2 {"email": "amani@exemple.cd"}"#);
    ok(&mut s, "commit");
    assert_eq!(ok(&mut s, "get user:2.email"), "user:2.email: amani@exemple.cd [string]");
    ok(&mut s, "begin");
    ok(&mut s, r#"set user:7 {"email": "chance@exemple.cd"}"#);
    ok(&mut s, r#"set user:8 {"email": "chance@exemple.cd"}"#);
    assert!(fails(&mut s, "commit").contains("already holds"));
    assert_eq!(ok(&mut s, "has user:7"), "No: Key 'user:7' does not exist.");
    // A deleted or expired document frees its value.
    ok(&mut s, "del user:1");
    ok(&mut s, r#"set user:9 {"email": "bisimwa@exemple.cd"} ttl 1"#);
    std::thread::sleep(Duration::from_millis(1100));
    ok(&mut s, r#"set user:10 {"email": "bisimwa@exemple.cd"}"#);

    // On several fields: the same name may come back under another parent.
    ok(&mut s, r#"set place:1 {"type": "VILLE", "parent": null, "nameLower": "goma"}"#);
    ok(&mut s, r#"set place:2 {"type": "QUARTIER", "parent": "place:1", "nameLower": "katindo"}"#);
    assert!(fails(&mut s, "create index on place: (type, parent)").contains("must be unique"));
    assert_eq!(
        ok(&mut s, "create unique index on place: (type, parent, nameLower)"),
        "Unique index created on place:* (type, parent, nameLower): 2 documents indexed."
    );
    ok(&mut s, r#"set place:3 {"type": "QUARTIER", "parent": "place:9", "nameLower": "katindo"}"#);
    let err = fails(&mut s, r#"set place:4 {"type": "QUARTIER", "parent": "place:1", "nameLower": "katindo"}"#);
    assert!(err.contains(r#"already holds ["QUARTIER","place:1","katindo"]"#), "{err}");
    // Two towns without a parent are two documents with the same values.
    assert!(fails(&mut s, r#"set place:5 {"type": "VILLE", "parent": null, "nameLower": "goma"}"#)
        .contains("already holds"));
    // A document lacking one of the fields is not constrained.
    ok(&mut s, r#"set place:5 {"type": "VILLE", "nameLower": "goma"}"#);
    // Queries do not use an index on several fields.
    assert!(ok(&mut s, r#"explain get where place:*.type = "VILLE""#).contains("PREFIX SCAN"));
    let list = ok(&mut s, "indexes");
    assert!(list.contains("place:* (type, parent, nameLower) - unique, 3 documents indexed"), "{list}");
    assert!(list.contains("user:* (email) - unique, hash, "), "{list}");

    // The constraint holds after a restart, and is dropped like any index.
    drop(s);
    let mut s = kiva(&dir);
    assert!(fails(&mut s, r#"set user:11 {"email": "amani@exemple.cd"}"#).contains("already holds"));
    assert!(fails(&mut s, r#"set place:6 {"type": "QUARTIER", "parent": "place:1", "nameLower": "katindo"}"#)
        .contains("already holds"));
    assert_eq!(
        ok(&mut s, "drop index on place: (type, parent, nameLower)"),
        "Index on place:* (type, parent, nameLower) dropped."
    );
    ok(&mut s, r#"set place:6 {"type": "QUARTIER", "parent": "place:1", "nameLower": "katindo"}"#);
    // A structured write answers the conflict like `set`.
    let err = kivadb::api::handle(
        &mut s,
        &serde_json::json!({"op": "put", "key": "user:12", "type": "object", "value": {"email": "amani@exemple.cd"}}),
    )
    .unwrap_err();
    assert!(matches!(err, kivadb::error::KivaError::Unique(_)), "{err}");
    assert!(err.to_string().starts_with("UNIQUE: 'user:12' not written"), "{err}");
}

#[test]
fn unique_indexes_may_ignore_case() {
    let dir = TempDir::new("unique-case");
    let mut s = kiva(&dir);
    ok(&mut s, r#"set place:1 {"type": "VILLE", "parent": null, "name": "Goma"}"#);
    ok(&mut s, r#"set place:2 {"type": "VILLE", "parent": null, "name": "GOMA"}"#);
    let err = fails(&mut s, "create unique index on place: (type, parent, name) ignore case");
    assert!(err.contains(r#"place:1 and place:2 both hold ["ville",null,"goma"]"#), "{err}");
    ok(&mut s, r#"update place:2 {"type": "VILLE", "parent": null, "name": "Bukavu"}"#);
    assert_eq!(
        ok(&mut s, "create unique index on place: (type, parent, name) ignore case"),
        "Unique index created on place:* (type, parent, name) ignore case: 2 documents indexed."
    );
    let err = fails(&mut s, r#"set place:3 {"type": "VILLE", "parent": null, "name": "gOmA"}"#);
    assert!(err.contains("Error: UNIQUE: 'place:3' not written: 'place:1' already holds"), "{err}");
    assert!(err.contains("unique index on place:* (type, parent, name) ignore case"), "{err}");
    // In every alphabet, and only for text.
    ok(&mut s, r#"set place:4 {"type": "VILLE", "parent": null, "name": "Élisabethville"}"#);
    assert!(fails(&mut s, r#"set place:5 {"type": "VILLE", "parent": null, "name": "élisabethville"}"#)
        .contains("already holds"));
    ok(&mut s, r#"set place:6 {"type": "VILLE", "parent": 1, "name": "Goma"}"#);
    // The documents keep their own text, and queries do not use this index.
    assert_eq!(ok(&mut s, "get place:1.name"), "place:1.name: Goma [string]");
    assert!(ok(&mut s, r#"explain get where place:*.name = "Goma""#).contains("PREFIX SCAN"));
    assert_eq!(ok(&mut s, r#"count where place:*.name = "goma""#), "Count: 0");
    assert!(ok(&mut s, "indexes").contains("place:* (type, parent, name) - unique, ignore case, 4 documents indexed"));
    assert!(fails(&mut s, "create index on user: (email) ignore case").contains("for unique indexes"));
    assert!(fails(&mut s, "create unique index on user: (email) ignore").contains("Usage"));
    drop(s);
    let mut s = kiva(&dir);
    assert!(
        fails(&mut s, r#"set place:7 {"type": "VILLE", "parent": null, "name": "BUKAVU"}"#).contains("already holds")
    );
    assert!(ok(&mut s, "indexes").contains("ignore case"));
}

#[test]
fn redis_clients_get_a_stable_error_from_unique_indexes() {
    use kivadb::resp::{Conn, Reply};
    let dir = TempDir::new("resp-unique");
    let inst = instance(&dir);
    let mut s = Session::new(inst.clone(), true, None);
    ok(&mut s, "create unique index on place: (type, parent, name) ignore case");
    ok(&mut s, r#"set place:1 {"type": "VILLE", "parent": null, "name": "Goma"}"#);
    let mut c = Conn::new(Session::new(inst, true, None));
    let call = |c: &mut Conn, args: &[&str]| c.handle(&args.iter().map(|a| a.as_bytes().to_vec()).collect::<Vec<_>>());
    let goma = r#"{"type": "VILLE", "parent": null, "name": "GOMA"}"#;

    let Reply::Error(e) = call(&mut c, &["JSON.SET", "place:2", "$", goma]) else { panic!("not refused") };
    assert!(e.starts_with("UNIQUE 'place:2' not written: 'place:1' already holds"), "{e}");
    // A hash without the parent field is not constrained.
    assert_eq!(call(&mut c, &["HSET", "place:3", "type", "VILLE", "name", "goma"]), Reply::Int(2));
    // A hash holds text: "null" is not the null of place:1.
    assert_eq!(call(&mut c, &["HSET", "place:3", "parent", "null"]), Reply::Int(1));
    // A hash is a document: HSET is checked like JSON.SET.
    let katindo = r#"{"type": "QUARTIER", "parent": "place:1", "name": "Katindo"}"#;
    assert_eq!(call(&mut c, &["JSON.SET", "place:7", "$", katindo]), Reply::Simple("OK".into()));
    let Reply::Error(e) =
        call(&mut c, &["HSET", "place:8", "type", "QUARTIER", "parent", "place:1", "name", "KATINDO"])
    else {
        panic!("HSET not refused")
    };
    assert!(e.starts_with("UNIQUE "), "{e}");

    // In MULTI, the write is queued; EXEC refuses the whole transaction.
    assert_eq!(call(&mut c, &["MULTI"]), Reply::Simple("OK".into()));
    assert!(matches!(call(&mut c, &["SET", "counter", "1"]), Reply::Simple(q) if q == "QUEUED"));
    call(&mut c, &["JSON.SET", "place:2", "$", goma]);
    let Reply::Error(e) = call(&mut c, &["EXEC"]) else { panic!("EXEC should fail") };
    assert!(e.starts_with("UNIQUE "), "{e}");
    assert_eq!(call(&mut c, &["EXISTS", "counter", "place:2"]), Reply::Int(0));

    // add through KIVA.API.
    let Reply::Bulk(Some(r)) = call(
        &mut c,
        &[
            "KIVA.API",
            r#"{"op": "add", "prefix": "place:", "value": {"type": "VILLE", "parent": null, "name": "Uvira"}}"#,
        ],
    ) else {
        panic!()
    };
    let r: serde_json::Value = serde_json::from_slice(&r).unwrap();
    assert!(r["key"].as_str().unwrap().starts_with("place:") && r["id"].as_str().unwrap().len() == 24, "{r}");
    let Reply::Error(e) = call(
        &mut c,
        &[
            "KIVA.API",
            r#"{"op": "add", "prefix": "place:", "value": {"type": "VILLE", "parent": null, "name": "uvira"}}"#,
        ],
    ) else {
        panic!()
    };
    assert!(e.starts_with("UNIQUE "), "{e}");
}

#[test]
fn add_names_a_new_key_after_its_prefix() {
    let dir = TempDir::new("add");
    let mut s = kiva(&dir);
    let key = |out: String| out.strip_prefix("OK: ").and_then(|o| o.strip_suffix(" saved.")).unwrap().to_string();
    let a = key(ok(&mut s, r#"add place: {"name": "Goma"}"#));
    let b = key(ok(&mut s, r#"add place: {"name": "Bukavu"} ttl 60"#));
    let id = a.strip_prefix("place:").unwrap();
    assert_eq!(id.len(), 24, "{a}");
    assert!(id.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()), "{a}");
    assert!(a < b, "later identifiers sort after: {a} {b}");
    assert_eq!(ok(&mut s, &format!("get {a}.name")), format!("{a}.name: Goma [string]"));
    assert!(ok(&mut s, &format!("scan {b}")).contains(&b));
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let secs = u64::from_str_radix(&id[..8], 16).unwrap();
    assert!(now.abs_diff(secs) <= 2, "the identifier starts with the time: {id}");
    ok(&mut s, r#"add counter 3"#);
    assert!(fails(&mut s, r#"add "place:" {}"#).contains("bare text"));
    assert!(fails(&mut s, "add place:").contains("Missing prefix or value"));
    assert!(fails(&mut s, r#"add pla*ce: {}"#).contains("forbidden character"));

    // In a transaction, the key exists at the commit.
    ok(&mut s, "begin");
    let c = key(ok(&mut s, r#"add place: {"name": "Uvira"}"#));
    ok(&mut s, "rollback");
    assert_eq!(ok(&mut s, &format!("has {c}")), format!("No: Key '{c}' does not exist."));

    let r =
        kivadb::api::handle(&mut s, &serde_json::json!({"op": "add", "prefix": "place:", "value": {"name": "Kindu"}}))
            .unwrap();
    let k = r["key"].as_str().unwrap();
    assert_eq!(k, format!("place:{}", r["id"].as_str().unwrap()));
    assert_eq!(r["type"], "object");
    assert_eq!(ok(&mut s, &format!("get {k}.name")), format!("{k}.name: Kindu [string]"));
    let err = kivadb::api::handle(&mut s, &serde_json::json!({"op": "add", "prefix": "n:", "value": 3})).unwrap_err();
    assert!(err.to_string().contains("\"type\" is needed"), "{err}");
    // Unique indexes apply to added documents.
    ok(&mut s, "create unique index on place: (name)");
    assert!(fails(&mut s, r#"add place: {"name": "Goma"}"#).contains("already holds"));
}
