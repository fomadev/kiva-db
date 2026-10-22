// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! Automatic failover, with real servers: the master process is killed.

mod common;

use common::TempDir;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// One KivaDB server of the group, in its own process.
struct Node {
    home: PathBuf,
    port: u16,
    child: Option<Child>,
}

impl Node {
    fn new(dir: &TempDir, name: &str) -> Node {
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let node = Node { home: dir.path().join(name), port, child: None };
        for setting in ["failover.auto true", "failover.heartbeat_ms 200", "failover.down_after_ms 1500"] {
            node.cli(&format!("config set {setting}"));
        }
        node
    }

    fn cli(&self, command: &str) -> String {
        let o = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(&self.home)
            .args(["-e", command])
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
    }

    fn serve(&mut self) {
        let child = Command::new(env!("CARGO_BIN_EXE_kivadb"))
            .arg("--home")
            .arg(&self.home)
            .args(["serve", "--port", &self.port.to_string(), "--no-http", "--no-resp"])
            .env("KIVADB_NO_UPDATE_CHECK", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        self.child = Some(child);
        wait_for("the server to listen", Duration::from_secs(20), || {
            std::net::TcpStream::connect(("127.0.0.1", self.port)).is_ok()
        });
    }

    /// Kills the process outright, as a power cut would.
    fn kill(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }

    fn is_master(&self) -> bool {
        self.cli("replica status").contains("Role: master")
    }

    fn follows(&self, master: &Node) -> bool {
        let s = self.cli("replica status");
        s.contains("Role: replica")
            && s.contains(&format!("Master: {}", master.addr()))
            && s.contains("Link: connected")
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        self.kill();
    }
}

fn wait_for(what: &str, limit: Duration, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn a_dead_master_is_replaced_and_steps_down_when_it_returns() {
    let dir = TempDir::new("failover_auto");
    let (mut a, mut b, mut c) = (Node::new(&dir, "a"), Node::new(&dir, "b"), Node::new(&dir, "c"));
    a.serve();
    b.serve();
    c.serve();
    assert!(b.cli(&format!("replica of 127.0.0.1 {}", a.port)).contains("Now replicating"));
    assert!(c.cli(&format!("replica of 127.0.0.1 {}", a.port)).contains("Now replicating"));
    a.cli(r#"set before "written on the first master""#);
    wait_for("both replicas to have the data", Duration::from_secs(20), || {
        b.cli("get before").contains("first master") && c.cli("get before").contains("first master")
    });
    // Each replica learns the other from the master's heartbeats.
    wait_for("the group to be known", Duration::from_secs(20), || {
        b.cli("failover status").contains(&c.addr()) && c.cli("failover status").contains(&b.addr())
    });
    let status = b.cli("failover status");
    assert!(status.contains("Automatic failover: on") && status.contains("2 of 3 nodes must agree"), "{status}");

    // The master dies. One replica takes over, the other follows it.
    a.kill();
    wait_for("a new master", Duration::from_secs(30), || b.is_master() || c.is_master());
    let (new, other) = if b.is_master() { (&b, &c) } else { (&c, &b) };
    wait_for("the other replica to follow the new master", Duration::from_secs(30), || other.follows(new));
    assert!(new.cli("failover status").contains("promoted to master in epoch 1"), "{}", new.cli("failover status"));
    assert!(new.cli("get before").contains("first master"));
    new.cli(r#"set after "written on the new master""#);
    wait_for("replication from the new master", Duration::from_secs(20), || {
        other.cli("get after").contains("new master")
    });
    // Clients writing to a replica are told where the master is.
    let refused = other.cli(r#"set nope "x""#);
    assert!(refused.contains(&format!("READONLY: this node is a replica of {}", new.addr())), "{refused}");

    // The old master comes back: it finds the newer epoch and steps down.
    a.serve();
    wait_for("the old master to follow the new one", Duration::from_secs(30), || a.follows(new));
    wait_for("the old master to resynchronise", Duration::from_secs(20), || a.cli("get after").contains("new master"));
    assert!(a.cli(r#"set nope "x""#).contains("READONLY"));
    assert!(a.cli("failover status").contains("Epoch: 1"));
}

#[test]
fn without_a_quorum_a_lone_replica_does_not_promote_itself() {
    let dir = TempDir::new("failover_quorum");
    let (mut a, mut b) = (Node::new(&dir, "a"), Node::new(&dir, "b"));
    a.serve();
    b.serve();
    b.cli(&format!("replica of 127.0.0.1 {}", a.port));
    a.cli("set k 1");
    wait_for("the replica to have the data", Duration::from_secs(20), || b.cli("get k").contains("k: 1"));
    a.kill();

    // Master + one replica: the replica alone is not a majority of two.
    wait_for("an election attempt", Duration::from_secs(20), || {
        b.cli("failover status").contains("only 1 of the 2 nodes needed see it down")
    });
    assert!(!b.is_master());

    // Accepting the risk explicitly lets it take over.
    b.cli("config set failover.quorum 1");
    wait_for("the promotion", Duration::from_secs(30), || b.is_master());
    assert!(b.cli("update k 2").contains("OK: k updated."));
}

/// A TCP relay to a node, that a test can cut like a network failure.
struct Proxy {
    port: u16,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    conns: std::sync::Arc<std::sync::Mutex<Vec<std::net::TcpStream>>>,
}

impl Proxy {
    fn to(target: u16) -> Proxy {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let conns: Arc<Mutex<Vec<std::net::TcpStream>>> = Arc::default();
        let (s, c) = (stop.clone(), conns.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::SeqCst) {
                let Ok((client, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                };
                let _ = client.set_nonblocking(false);
                let Ok(upstream) = std::net::TcpStream::connect(("127.0.0.1", target)) else { continue };
                for (mut from, mut to) in [
                    (client.try_clone().unwrap(), upstream.try_clone().unwrap()),
                    (upstream.try_clone().unwrap(), client.try_clone().unwrap()),
                ] {
                    std::thread::spawn(move || {
                        let _ = std::io::copy(&mut from, &mut to);
                        let _ = to.shutdown(std::net::Shutdown::Both);
                    });
                }
                c.lock().unwrap().extend([client, upstream]);
            }
        });
        Proxy { port, stop, conns }
    }

    /// The link fails: open connections break, new ones are refused.
    fn cut(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
        for c in self.conns.lock().unwrap().drain(..) {
            let _ = c.shutdown(std::net::Shutdown::Both);
        }
    }
}

#[test]
fn a_replica_cut_off_from_the_master_alone_does_not_take_over() {
    let dir = TempDir::new("failover_split");
    let (mut a, mut b, mut c) = (Node::new(&dir, "a"), Node::new(&dir, "b"), Node::new(&dir, "c"));
    a.serve();
    b.serve();
    c.serve();
    // B reaches the master through a link that will fail; C directly.
    let link = Proxy::to(a.port);
    b.cli(&format!("replica of 127.0.0.1 {}", link.port));
    c.cli(&format!("replica of 127.0.0.1 {}", a.port));
    a.cli("set k 1");
    wait_for("the group to be known", Duration::from_secs(20), || {
        b.cli("get k").contains("k: 1") && b.cli("failover status").contains(&c.addr())
    });

    link.cut();
    // B sees the master down, C does not: no majority, no second master.
    wait_for("an election attempt", Duration::from_secs(20), || {
        b.cli("failover status").contains("only 1 of the 2 nodes needed see it down")
    });
    std::thread::sleep(Duration::from_secs(3));
    assert!(a.is_master() && !b.is_master() && !c.is_master());
    assert!(c.follows(&a));
    a.cli("set k2 2");
    wait_for("the master to keep serving its replicas", Duration::from_secs(10), || c.cli("get k2").contains("k2: 2"));
}
