mod common;

use common::{fails, instance, ok, run, TempDir};
use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use std::sync::mpsc;
use std::time::{Duration, Instant};

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
    let ttl = s.engine.meta("sess").unwrap().expires_at - kivadb::util::now_secs();
    assert!((148..=150).contains(&ttl), "ttl was {ttl}");
    ok(&mut s, "bump string sess set 10");
    let ttl = s.engine.meta("sess").unwrap().expires_at - kivadb::util::now_secs();
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
    assert_eq!(ok(&mut s, "whoami"), "fordi (editor) @ workspace 'u_fordi'");

    // Own workspace: isolated from the default one.
    assert_eq!(ok(&mut s, "get shared"), "shared: (nil)");
    ok(&mut s, r#"set mine "private""#);
    assert!(fails(&mut s, "workspace switch default").contains("no access"));
    // Maintenance of one's own workspace is allowed; of the server, not.
    assert!(ok(&mut s, "compact").contains("compacted successfully"));
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
    assert!(list.contains("default") && list.contains("u_fordi") && !list.contains("u_guest"));
    ok(&mut admin, r#"grant write on all to "fordi""#);
    ok(&mut s, "workspace switch default");
    ok(&mut s, "set yes 1");
    // Writing somewhere does not make it one's own: compacting it stays an admin task.
    assert!(fails(&mut s, "compact").contains("except in your personal workspace 'u_fordi'"));
    ok(&mut admin, r#"revoke write on all from "fordi""#);
    ok(&mut s, "workspace switch default");
    assert!(fails(&mut s, "set again 1").contains("only have read access"));

    // Viewers can read but never write.
    let mut g = Session::new(inst.clone(), false, None);
    ok(&mut g, "auth guest g");
    assert!(fails(&mut g, "set k 1").contains("viewer role is read-only"));
    assert!(fails(&mut g, "eval 'return 1'").contains("viewer role is read-only"));
    assert!(fails(&mut g, "compact").contains("viewer role is read-only"));
    ok(&mut admin, r#"modify user "guest" role "editor""#);
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
