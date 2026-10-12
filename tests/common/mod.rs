#![allow(dead_code)]

use kivadb::instance::{Instance, Options};
use kivadb::session::Session;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, empty directory that is removed when the guard is dropped.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(tag: &str) -> TempDir {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("kivadb_test_{tag}_{}_{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }

    pub fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub fn instance(dir: &TempDir) -> Arc<Instance> {
    Instance::open(dir.path(), Options::default()).unwrap()
}

/// Runs a command and returns its output as one string.
pub fn run(sess: &mut Session, cmd: &str) -> String {
    sess.execute(cmd).text()
}

/// Runs a command that is expected to succeed.
pub fn ok(sess: &mut Session, cmd: &str) -> String {
    let out = sess.execute(cmd);
    assert!(!out.has_err(), "`{cmd}` failed: {}", out.text());
    out.text()
}

/// Runs a command that is expected to fail and returns the error text.
pub fn fails(sess: &mut Session, cmd: &str) -> String {
    let out = sess.execute(cmd);
    assert!(out.has_err(), "`{cmd}` should have failed but printed: {}", out.text());
    out.text()
}
