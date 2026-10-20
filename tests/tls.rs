// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Encrypted connections: every port accepts TLS next to plain connections.

mod common;

use common::{instance, ok, TempDir};
use kivadb::client::Client;
use kivadb::session::Session;
use kivadb::{http, resp, server, tls};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
    let start = Instant::now();
    while !cond() {
        assert!(start.elapsed() < Duration::from_secs(10), "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_binary_protocol_works_over_tls_with_a_pinned_certificate() {
    let dir = TempDir::new("tls_binary");
    let inst = instance(&dir);
    let port = server::start_tcp(&inst, "127.0.0.1", 0).unwrap();
    let fingerprint = inst.tls().expect("a certificate is created on first use").fingerprint.clone();
    assert!(dir.path().join("tls").join("cert.pem").exists());

    let mut db = Client::connect_tls(&format!("127.0.0.1:{port}"), Some(&fingerprint)).unwrap();
    assert_eq!(db.server_fingerprint.as_deref(), Some(fingerprint.as_str()));
    db.set("secret", &kivadb::Value::Str("over tls".into()), 0).unwrap();
    assert_eq!(db.exec("get secret").unwrap(), "secret: over tls [string]");
    // Large values cross the relay intact.
    let big = "x".repeat(300_000);
    db.set("big", &kivadb::Value::Str(big.clone()), 0).unwrap();
    assert_eq!(db.get("big").unwrap(), Some(kivadb::Value::Str(big)));

    // Without a fingerprint the certificate is accepted and reported...
    let probe = Client::connect_tls(&format!("127.0.0.1:{port}"), None).unwrap();
    assert_eq!(probe.server_fingerprint.as_deref(), Some(fingerprint.as_str()));
    // ...and a different one is refused before anything is sent.
    let wrong = format!("{}AB", "AB:".repeat(31));
    let err = Client::connect_tls(&format!("127.0.0.1:{port}"), Some(&wrong)).err().unwrap().to_string();
    assert!(err.contains("not the one expected"), "{err}");

    // Plain connections still work next to encrypted ones.
    let mut plain = Client::connect(("127.0.0.1", port)).unwrap();
    assert_eq!(plain.exec("get secret").unwrap(), "secret: over tls [string]");
}

#[test]
fn https_and_redis_over_tls_on_their_usual_ports() {
    let dir = TempDir::new("tls_http");
    let inst = instance(&dir);
    let port = http::start_http(&inst, "127.0.0.1", 0).unwrap();
    let fp = inst.tls().unwrap().fingerprint.clone();
    let mut s = tls::connect(&format!("127.0.0.1:{port}"), Some(&fp), Duration::from_secs(5)).unwrap().stream;
    s.write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
    let mut answer = String::new();
    s.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 200") && answer.contains("\"status\":\"ok\""), "{answer}");

    let rport = resp::start_resp(&inst, "127.0.0.1", 0).unwrap();
    let mut r = tls::connect(&format!("127.0.0.1:{rport}"), Some(&fp), Duration::from_secs(5)).unwrap().stream;
    r.write_all(b"*3\r\n$3\r\nSET\r\n$1\r\nk\r\n$1\r\nv\r\n*2\r\n$3\r\nGET\r\n$1\r\nk\r\n").unwrap();
    let mut lines = BufReader::new(r);
    let mut line = String::new();
    lines.read_line(&mut line).unwrap();
    assert_eq!(line, "+OK\r\n");
    line.clear();
    lines.read_line(&mut line).unwrap();
    assert_eq!(line, "$1\r\n");
}

#[test]
fn a_configured_certificate_is_used_and_tls_can_be_turned_off() {
    let dir = TempDir::new("tls_config");
    let inst = instance(&dir);
    let created = inst.tls().unwrap().fingerprint.clone();
    // A certificate configured by path is read from there.
    let home = dir.path();
    let copy = home.join("mine");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(home.join("tls/cert.pem"), copy.join("c.pem")).unwrap();
    std::fs::copy(home.join("tls/key.pem"), copy.join("k.pem")).unwrap();
    let mut admin = Session::new(inst.clone(), true, None);
    ok(&mut admin, &format!(r#"config set tls.cert "{}""#, copy.join("c.pem").display()));
    ok(&mut admin, &format!(r#"config set tls.key "{}""#, copy.join("k.pem").display()));
    let loaded = tls::load_server(home, &inst.cfg()).unwrap();
    assert_eq!(loaded.fingerprint, created);
    assert_eq!(loaded.cert_path, copy.join("c.pem"));
    assert!(ok(&mut admin, "config set tls.required true").contains("tls.required = true"));
    assert!(common::fails(&mut admin, "config set tls.required maybe").contains("invalid value"));
}

#[test]
fn replicas_follow_their_master_over_tls() {
    let (dir_m, dir_r) = (TempDir::new("tls_master"), TempDir::new("tls_replica"));
    let (master, replica) = (instance(&dir_m), instance(&dir_r));
    let port_m = server::start_tcp(&master, "127.0.0.1", 0).unwrap();
    server::start_tcp(&replica, "127.0.0.1", 0).unwrap();
    let fp = master.tls().unwrap().fingerprint.clone();
    let mut m = Session::new(master.clone(), true, None);
    let mut r = Session::new(replica.clone(), true, None);
    ok(&mut m, r#"set seed "from the master""#);

    ok(&mut r, "config set master_tls true");
    ok(&mut r, &format!(r#"config set master_fingerprint "{fp}""#));
    ok(&mut r, &format!("replica of 127.0.0.1 {port_m}"));
    wait_for("the encrypted link", || replica.repl.link().status == "connected");
    assert_eq!(ok(&mut r, "get seed"), "seed: from the master [string]");
    ok(&mut m, r#"set live "streamed""#);
    assert_eq!(ok(&mut m, "wait 1"), "OK: 1 replica(s) acknowledged.");
    assert_eq!(ok(&mut r, "get live"), "live: streamed [string]");
}
