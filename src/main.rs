// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

use kivadb::config::default_home;
use kivadb::instance::{Instance, Options, VERSION};
use kivadb::shell::{self, ShellOpts};
use kivadb::util::is_loopback;
use kivadb::{auth, http, repl, server, update, KivaError};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "\
KivaDB - typed key-value and document database

USAGE:
  kivadb start [OPTIONS]      Start the database in the background
  kivadb stop                 Stop the running database
  kivadb status               Tell whether the database is running
  kivadb update [--check]     Install the latest release (--check: only tell)
  kivadb run <file.kva>       Run the commands of a file (--keep-going: do not stop at an error)
  kivadb tls [renew]          Show the server's TLS certificate (renew: create a new one)
  kivadb [OPTIONS]            Open the interactive shell
  kivadb serve [OPTIONS]      Run the server in this terminal (Ctrl+C to stop)

SHELL OPTIONS:
  -w, --workspace <name>      Workspace to open (default: your home workspace)
  -u, --user <name>           Log in as this user
  -p, --password <password>   Password (prompted when omitted)
  -e <command>                Run a command and exit (repeatable)
      --connect <host:port>   Connect to a running server instead of opening files
      --tls                   Encrypt that connection (TLS); a new server is shown first
      --tls-fingerprint <fp>  Accept only the server certificate with this SHA-256
      --no-listen             Do not accept other local shells on this instance

SERVER OPTIONS (start, serve):
      --port <n>              TCP port (default: [network] port, 2911)
      --bind <address>        Listen address (default: [network] bind, 127.0.0.1)
      --http-port <n>         REST API port (default: [http] port, 8080)
      --no-http               Do not start the REST API
      --resp-port <n>         Redis-compatible port (default: [resp] port, 6379)
      --no-resp               Do not start the Redis-compatible listener

COMMON OPTIONS:
      --home <dir>            KivaDB home directory (config, users, data)
      --skip-auth             Start without authentication (maintenance only)
      --lua-debug             Trace Lua scripts line by line
  -V, --version               Print the version
  -h, --help                  Print this help
";

#[derive(Default)]
struct Args {
    serve: bool,
    action: Option<&'static str>,
    home: Option<PathBuf>,
    connect: Option<String>,
    no_listen: bool,
    check_only: bool,
    renew: bool,
    run_file: Option<PathBuf>,
    port: Option<u16>,
    bind: Option<String>,
    http_port: Option<u16>,
    no_http: bool,
    no_resp: bool,
    resp_port: Option<u16>,
    opts: Options,
    shell: ShellOpts,
}

fn fail(msg: &str) -> ! {
    eprintln!("Error: {msg}");
    exit(2);
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    let value = |it: &mut dyn Iterator<Item = String>, flag: &str| {
        it.next().unwrap_or_else(|| fail(&format!("{flag} needs a value. Try --help.")))
    };
    let port = |s: String, flag: &str| {
        s.parse::<u16>().ok().filter(|p| *p != 0).unwrap_or_else(|| fail(&format!("{flag} needs a port number.")))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "serve" | "server" => args.serve = true,
            "start" => args.action = Some("start"),
            "stop" => args.action = Some("stop"),
            "status" => args.action = Some("status"),
            "update" => args.action = Some("update"),
            "tls" => args.action = Some("tls"),
            "renew" => args.renew = true,
            "run" => args.run_file = Some(PathBuf::from(value(&mut it, "run"))),
            "--keep-going" => args.shell.keep_going = true,
            "--tls" => args.shell.tls = true,
            "--tls-fingerprint" => args.shell.tls_fingerprint = Some(value(&mut it, "--tls-fingerprint")),
            "--check" => args.check_only = true,
            "shell" => {}
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                exit(0);
            }
            "-V" | "--version" | "version" => {
                println!("KivaDB {VERSION}");
                exit(0);
            }
            "--home" => args.home = Some(PathBuf::from(value(&mut it, "--home"))),
            "-w" | "--workspace" => args.shell.workspace = Some(value(&mut it, "--workspace")),
            "-u" | "--user" => args.shell.user = Some(value(&mut it, "--user")),
            "-p" | "--password" => args.shell.password = Some(value(&mut it, "--password")),
            "-e" => args.shell.commands.push(value(&mut it, "-e")),
            "--connect" => args.connect = Some(value(&mut it, "--connect")),
            "--no-listen" => args.no_listen = true,
            "--skip-auth" => args.opts.skip_auth = true,
            "--lua-debug" => args.opts.lua_debug = true,
            "--port" => args.port = Some(port(value(&mut it, "--port"), "--port")),
            "--http-port" => args.http_port = Some(port(value(&mut it, "--http-port"), "--http-port")),
            "--bind" => args.bind = Some(value(&mut it, "--bind")),
            "--no-http" => args.no_http = true,
            "--no-resp" => args.no_resp = true,
            "--resp-port" => args.resp_port = Some(port(value(&mut it, "--resp-port"), "--resp-port")),
            other => fail(&format!("unknown argument '{other}'. Try --help.")),
        }
    }
    args
}

fn on_ctrl_c(inst: &Arc<Instance>, stop: Option<Arc<AtomicBool>>) {
    let inst = inst.clone();
    let _ = ctrlc::set_handler(move || match &stop {
        // The server leaves its main loop and shuts down in order.
        Some(flag) => flag.store(true, Ordering::SeqCst),
        None => {
            inst.shutdown();
            exit(130);
        }
    });
}

fn serve(inst: Arc<Instance>, args: &Args) -> i32 {
    // Settings may also come from the environment, which is how containers
    // and service managers configure a program. Command-line flags win.
    let env = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
    let env_port = |name: &str| env(name).and_then(|v| v.trim().parse::<u16>().ok()).filter(|p| *p != 0);
    let (bind, port, http_on, http_port) = {
        let c = inst.cfg();
        (
            args.bind.clone().or_else(|| env("KIVADB_BIND")).unwrap_or_else(|| c.get("network", "bind")),
            args.port.or_else(|| env_port("KIVADB_PORT")).unwrap_or(c.get_u64("network", "port") as u16),
            c.get_bool("http", "enabled") && !args.no_http,
            args.http_port.or_else(|| env_port("KIVADB_HTTP_PORT")).unwrap_or(c.get_u64("http", "port") as u16),
        )
    };
    // First start of a hosted instance: KIVADB_ADMIN_PASSWORD secures it in
    // one step. A password that was already set is never overwritten.
    if let Some(password) = env("KIVADB_ADMIN_PASSWORD") {
        let secured = (|| -> kivadb::Result<bool> {
            let mut changed = false;
            if let Some(mut admin) = auth::get_user(&inst.system, auth::DEFAULT_ADMIN)? {
                if admin.has_empty_password() {
                    admin.set_password(&password);
                    auth::save_user(&inst.system, &admin)?;
                    inst.audit.log("system", "password of kiva set from KIVADB_ADMIN_PASSWORD", true);
                    changed = true;
                }
            }
            if !inst.cfg().get_bool("security", "auth_required") {
                inst.cfg_mut().set("security", "auth_required", "true")?;
                changed = true;
            }
            Ok(changed)
        })();
        match secured {
            Ok(true) => println!("  Security     : administrator password set, authentication turned on"),
            Ok(false) => {}
            Err(e) => {
                eprintln!("Error: cannot apply KIVADB_ADMIN_PASSWORD: {e}");
                return 1;
            }
        }
    }
    // Never expose an unauthenticated database to the network.
    if !is_loopback(&bind) {
        let root_open =
            auth::get_user(&inst.system, auth::DEFAULT_ADMIN).ok().flatten().is_some_and(|u| u.has_empty_password());
        if !inst.auth_required() || root_open {
            eprintln!("Error: refusing to listen on {bind} without authentication.");
            eprintln!("  1. kivadb -e \"modify user \\\"kiva\\\" password \\\"<a strong password>\\\"\"");
            eprintln!("  2. kivadb -e \"config set auth_required true\"");
            return 1;
        }
    }
    let tcp_port = match server::start_tcp(&inst, &bind, port) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    println!("KivaDB {VERSION} server");
    println!("  Home         : {}", inst.home.display());
    println!("  TCP protocol : {bind}:{tcp_port} (binary and plain text)");
    let tls_required = inst.cfg().get_bool("tls", "required");
    match inst.tls() {
        Some(t) => println!(
            "  TLS          : on every port{}, certificate SHA-256 {}",
            if tls_required { ", required from other machines" } else { "" },
            t.fingerprint
        ),
        None => println!("  TLS          : off"),
    }
    if !is_loopback(&bind) && !tls_required {
        eprintln!(
            "[WARN] other machines may still connect without encryption. To refuse it: config set tls.required true"
        );
    }
    if http_on {
        match http::start_http(&inst, &bind, http_port) {
            Ok(p) => println!(
                "  REST API     : http://{bind}:{p}  (docs: /docs, metrics: /metrics, events: /ws, /v1/stream)"
            ),
            Err(e) => eprintln!("[WARN] REST API not started: {e}"),
        }
    }
    let (resp_on, resp_port) = {
        let c = inst.cfg();
        (
            c.get_bool("resp", "enabled") && !args.no_resp,
            args.resp_port.or_else(|| env_port("KIVADB_RESP_PORT")).unwrap_or(c.get_u64("resp", "port") as u16),
        )
    };
    if resp_on {
        match kivadb::resp::start_resp(&inst, &bind, resp_port) {
            Ok(p) => println!("  Redis (RESP) : {bind}:{p}  (redis-cli and Redis client libraries)"),
            Err(e) => eprintln!("[WARN] Redis-compatible listener not started: {e}"),
        }
    }
    println!("  Auth         : {}", if inst.auth_required() { "required" } else { "disabled (local use)" });
    if inst.opts.skip_auth {
        eprintln!("[WARN] Authentication disabled - unsafe for production");
    }
    let (role, host, mport) = {
        let c = inst.cfg();
        (
            c.get("replication", "role"),
            c.get("replication", "master_host"),
            c.get_u64("replication", "master_port") as u16,
        )
    };
    if matches!(role.as_str(), "replica" | "slave") && !host.is_empty() {
        repl::start_replica(&inst, &host, mport);
        println!("  Replication  : replica of {host}:{mport}");
    } else {
        println!("  Replication  : master");
    }
    println!("Press Ctrl+C to stop.");

    let stop = Arc::new(AtomicBool::new(false));
    on_ctrl_c(&inst, Some(stop.clone()));
    while !stop.load(Ordering::SeqCst) && !inst.stop_requested.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    // Whoever was reading our output may be gone: never fail on this line.
    let _ = std::io::Write::write_all(
        &mut std::io::stdout(),
        b"Shutting down...
",
    );
    inst.shutdown();
    0
}

/// Port of the KivaDB process that currently owns `home`, if there is one.
fn running_port(home: &Path) -> Option<u16> {
    let lock = std::fs::OpenOptions::new().write(true).open(home.join("instance.lock")).ok()?;
    if lock.try_lock().is_ok() {
        // Nobody holds the lock: whatever the port file says is stale.
        return None;
    }
    std::fs::read_to_string(home.join("instance.port")).ok()?.trim().parse().ok()
}

fn running_pid(home: &Path) -> String {
    std::fs::read_to_string(home.join("instance.pid")).map(|p| p.trim().to_string()).unwrap_or_else(|_| "?".into())
}

/// `kivadb tls`: the certificate clients should expect, created if needed.
fn show_tls(home: &Path, renew: bool) -> i32 {
    let cfg = match kivadb::config::Config::load(home) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    if !cfg.get_bool("tls", "enabled") {
        println!("TLS is off (tls.enabled = false).");
        return 0;
    }
    if renew {
        if !cfg.get("tls", "cert").trim().is_empty() {
            eprintln!("Error: the certificate is configured (tls.cert): replace its files instead.");
            return 1;
        }
        for f in ["cert.pem", "key.pem"] {
            let _ = std::fs::remove_file(home.join("tls").join(f));
        }
        println!("A new certificate was created. Restart KivaDB to use it; clients will see a new fingerprint.");
    }
    match kivadb::tls::load_server(home, &cfg) {
        Ok(t) => {
            println!("Certificate : {}", t.cert_path.display());
            println!("SHA-256     : {}", t.fingerprint);
            println!(
                "Encryption  : {}",
                if cfg.get_bool("tls", "required") {
                    "required for connections from other machines"
                } else {
                    "accepted; plain connections are accepted too (config set tls.required true to refuse them)"
                }
            );
            println!("Clients check this fingerprint the first time they connect.");
            0
        }
        Err(e) => {
            eprintln!("Error: {e}");
            1
        }
    }
}

/// The other listeners the server announced in its log (REST API, Redis
/// protocol), or why they did not start. The TCP port is known first: the
/// rest of the banner follows within moments.
fn listeners(log_path: &Path, from: u64) -> Vec<String> {
    let mut text = String::new();
    for _ in 0..30 {
        text = std::fs::read(log_path)
            .ok()
            .and_then(|b| b.get(from as usize..).map(|b| String::from_utf8_lossy(b).into_owned()))
            .unwrap_or_default();
        if text.contains("Press Ctrl+C to stop.") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("  REST API     : ") {
            out.push(format!("  REST API : {}", rest.split_whitespace().next().unwrap_or(rest)));
        } else if let Some(rest) = line.strip_prefix("  Redis (RESP) : ") {
            let address = rest.split_whitespace().next().unwrap_or(rest);
            out.push(format!("  Redis    : port {}", address.rsplit(':').next().unwrap_or(address)));
        } else if let Some(rest) = line.strip_prefix("  TLS          : ") {
            out.push(format!("  TLS      : {rest}"));
        } else if line.starts_with("[WARN] REST API") || line.starts_with("[WARN] Redis") {
            out.push(format!("  {line}"));
        }
    }
    out
}

/// `kivadb start`: launches `kivadb serve` detached from this terminal.
fn start(home: &Path, args: &Args) -> i32 {
    if let Some(port) = running_port(home) {
        println!("KivaDB is already running (pid {}) on port {port}.", running_pid(home));
        return 0;
    }
    if let Err(e) = std::fs::create_dir_all(home) {
        eprintln!("Error: cannot create {}: {e}", home.display());
        return 1;
    }
    let log_path = home.join("server.log");
    // The server appends to this log; what it writes from here on is ours.
    let log_start = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);
    let log = std::fs::OpenOptions::new().create(true).append(true).open(&log_path);
    let (Ok(exe), Ok(log)) = (std::env::current_exe(), log) else {
        eprintln!("Error: cannot prepare the server process.");
        return 1;
    };
    let Ok(log_err) = log.try_clone() else { return 1 };
    let mut cmd = Command::new(exe);
    cmd.arg("serve").arg("--home").arg(home);
    if let Some(p) = args.port {
        cmd.arg("--port").arg(p.to_string());
    }
    if let Some(p) = args.http_port {
        cmd.arg("--http-port").arg(p.to_string());
    }
    if let Some(b) = &args.bind {
        cmd.arg("--bind").arg(b);
    }
    if args.no_http {
        cmd.arg("--no-http");
    }
    if args.no_resp {
        cmd.arg("--no-resp");
    }
    if let Some(p) = args.resp_port {
        cmd.arg("--resp-port").arg(p.to_string());
    }
    if args.opts.skip_auth {
        cmd.arg("--skip-auth");
    }
    if args.opts.lua_debug {
        cmd.arg("--lua-debug");
    }
    cmd.stdin(Stdio::null()).stdout(log).stderr(log_err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Its own process group: closing the terminal (or the SSH session)
        // that ran `kivadb start` must not stop the server with it.
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use std::os::windows::process::CommandExt;
        extern "system" {
            fn SetHandleInformation(handle: *mut std::ffi::c_void, mask: u32, flags: u32) -> i32;
        }
        // The server must not inherit this terminal's handles: whoever is
        // waiting for `kivadb start` to finish would wait for the server too.
        let handles =
            [std::io::stdin().as_raw_handle(), std::io::stdout().as_raw_handle(), std::io::stderr().as_raw_handle()];
        for h in handles {
            // SAFETY: plain Win32 call on our own standard handles; a null or
            // invalid handle makes it fail harmlessly.
            unsafe { SetHandleInformation(h.cast(), 1, 0) };
        }
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP: survives this terminal.
        cmd.creation_flags(0x0800_0000 | 0x0000_0200);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: cannot start the server: {e}");
            return 1;
        }
    };
    for _ in 0..100 {
        if let Some(port) = running_port(home) {
            println!("KivaDB started (pid {}).", child.id());
            println!("  TCP port : {port}");
            for line in listeners(&log_path, log_start) {
                println!("{line}");
            }
            println!("  Home     : {}", home.display());
            println!("  Log      : {}", log_path.display());
            println!("Open a shell with 'kivadb', stop the database with 'kivadb stop'.");
            return 0;
        }
        if let Ok(Some(status)) = child.try_wait() {
            eprintln!("Error: the server stopped right away ({status}). Last lines of {}:", log_path.display());
            let text = std::fs::read_to_string(&log_path).unwrap_or_default();
            for l in text.lines().rev().take(6).collect::<Vec<_>>().into_iter().rev() {
                eprintln!("  {l}");
            }
            return 1;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("Error: the server did not come up within 10 seconds. See {}.", log_path.display());
    1
}

fn stop(home: &Path, args: &Args) -> i32 {
    let Some(port) = running_port(home) else {
        println!("KivaDB is not running.");
        return 0;
    };
    let mut opts = args.shell.clone();
    opts.commands = vec!["shutdown".to_string()];
    if shell::run_client(&format!("127.0.0.1:{port}"), opts) != 0 {
        return 1;
    }
    for _ in 0..100 {
        if running_port(home).is_none() {
            println!("KivaDB stopped.");
            return 0;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("Error: KivaDB is still shutting down after 10 seconds.");
    1
}

fn status(home: &Path) -> i32 {
    match running_port(home) {
        Some(port) => {
            println!("KivaDB is running (pid {}).", running_pid(home));
            println!("  TCP port : {port}");
            println!("  Home     : {}", home.display());
            0
        }
        None => {
            println!("KivaDB is not running. Start it with: kivadb start");
            1
        }
    }
}

/// Ends the process, after telling a person at a terminal about a new release.
fn finish(notice: &Option<update::Notice>, code: i32) -> ! {
    if let Some(n) = notice {
        n.finish();
    }
    exit(code);
}

fn main() {
    update::clean_previous();
    let mut args = parse_args();
    let home = args.home.clone().unwrap_or_else(default_home);
    // The interactive history lives next to the data (or in the default home for --connect).
    args.shell.history = Some(home.join("history.txt"));
    args.shell.home = Some(home.clone());
    if let Some(file) = &args.run_file {
        match kivadb::kva::load(file) {
            Ok(statements) => args.shell.script = Some((file.display().to_string(), statements)),
            Err(e) => fail(&e),
        }
    }
    if args.action == Some("update") {
        exit(update::run(args.check_only, &home));
    }
    if args.action == Some("tls") {
        exit(show_tls(&home, args.renew));
    }
    // A server runs unattended: nobody would read the notice.
    let notice = if args.serve { None } else { update::Notice::begin(&home) };
    if let Some(addr) = &args.connect {
        finish(&notice, shell::run_client(addr, args.shell.clone()));
    }
    match args.action {
        Some("start") => finish(&notice, start(&home, &args)),
        Some("stop") => finish(&notice, stop(&home, &args)),
        Some("status") => finish(&notice, status(&home)),
        _ => {}
    }
    let inst = match Instance::open(&home, args.opts.clone()) {
        Ok(i) => i,
        Err(KivaError::Locked(_)) if !args.serve => {
            // KivaDB is already running on this home: join it instead of
            // fighting over the files, so every terminal sees the same data.
            match running_port(&home) {
                Some(p) => finish(&notice, shell::run_client(&format!("127.0.0.1:{p}"), args.shell.clone())),
                None => fail(&format!(
                    "{} is in use by another KivaDB process that does not accept connections.",
                    home.display()
                )),
            }
        }
        Err(KivaError::Locked(_)) => {
            fail(&format!("KivaDB is already running on {} (see 'kivadb status').", home.display()))
        }
        Err(e) => fail(&e.to_string()),
    };
    if args.serve {
        exit(serve(inst, &args));
    }
    if !args.no_listen {
        // While this shell is open the database is reachable like a started
        // one: on the configured port if it is free, on any free port otherwise.
        // One-shot `-e` runs stay out of the way and take any free port.
        let port = if args.shell.commands.is_empty() { inst.cfg().get_u64("network", "port") as u16 } else { 0 };
        let listening =
            server::start_tcp(&inst, "127.0.0.1", port).or_else(|_| server::start_tcp(&inst, "127.0.0.1", 0));
        if let Err(e) = listening {
            eprintln!("[WARN] other programs will not be able to connect: {e}");
        }
    }
    on_ctrl_c(&inst, None);
    {
        // A `shutdown` sent by another terminal ends this process too.
        let inst = inst.clone();
        std::thread::spawn(move || loop {
            if inst.stop_requested.load(Ordering::SeqCst) {
                inst.shutdown();
                exit(0);
            }
            std::thread::sleep(Duration::from_millis(200));
        });
    }
    let code = shell::run_embedded(inst.clone(), args.shell.clone());
    inst.shutdown();
    finish(&notice, code);
}
