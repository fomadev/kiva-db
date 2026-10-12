// Copyright (c) 2026 Fordi / FomaDev.
// Licensed under FomaDev Public License.
// See LICENSE file in the project root for full license information.

use kivadb::config::default_home;
use kivadb::instance::{Instance, Options, VERSION};
use kivadb::shell::{self, ShellOpts};
use kivadb::util::is_loopback;
use kivadb::{auth, http, repl, server, KivaError};
use std::path::PathBuf;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "\
KivaDB - typed key-value and document database

USAGE:
  kivadb [OPTIONS]            Open the interactive shell
  kivadb serve [OPTIONS]      Run the server (TCP protocol + REST API)

SHELL OPTIONS:
  -w, --workspace <name>      Workspace to open (default: your home workspace)
  -u, --user <name>           Log in as this user
  -p, --password <password>   Password (prompted when omitted)
  -e <command>                Run a command and exit (repeatable)
      --connect <host:port>   Connect to a running server instead of opening files
      --no-listen             Do not accept other local shells on this instance

SERVER OPTIONS:
      --port <n>              TCP port (default: [network] port, 2911)
      --bind <address>        Listen address (default: [network] bind, 127.0.0.1)
      --http-port <n>         REST API port (default: [http] port, 8080)
      --no-http               Do not start the REST API

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
    home: Option<PathBuf>,
    connect: Option<String>,
    no_listen: bool,
    port: Option<u16>,
    bind: Option<String>,
    http_port: Option<u16>,
    no_http: bool,
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
    let (bind, port, http_on, http_port) = {
        let c = inst.cfg();
        (
            args.bind.clone().unwrap_or_else(|| c.get("network", "bind")),
            args.port.unwrap_or(c.get_u64("network", "port") as u16),
            c.get_bool("http", "enabled") && !args.no_http,
            args.http_port.unwrap_or(c.get_u64("http", "port") as u16),
        )
    };
    // Never expose an unauthenticated database to the network.
    if !is_loopback(&bind) {
        let root_open = auth::get_user(&inst.system, "root").ok().flatten().is_some_and(|u| u.has_empty_password());
        if !inst.auth_required() || root_open {
            eprintln!("Error: refusing to listen on {bind} without authentication.");
            eprintln!("  1. kivadb -e \"modify user \\\"root\\\" password \\\"<a strong password>\\\"\"");
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
    if http_on {
        match http::start_http(&inst, &bind, http_port) {
            Ok(p) => println!("  REST API     : http://{bind}:{p}  (docs: /docs, metrics: /metrics, events: /ws)"),
            Err(e) => eprintln!("[WARN] REST API not started: {e}"),
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
    while !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("Shutting down...");
    inst.shutdown();
    0
}

fn main() {
    let args = parse_args();
    if let Some(addr) = &args.connect {
        exit(shell::run_client(addr, args.shell.clone()));
    }
    let home = args.home.clone().unwrap_or_else(default_home);
    let inst = match Instance::open(&home, args.opts.clone()) {
        Ok(i) => i,
        Err(KivaError::Locked(_)) if !args.serve => {
            // Another KivaDB process owns this home: join it instead of
            // fighting over the files, so every terminal sees the same data.
            let port =
                std::fs::read_to_string(home.join("instance.port")).ok().and_then(|p| p.trim().parse::<u16>().ok());
            match port {
                Some(p) => exit(shell::run_client(&format!("127.0.0.1:{p}"), args.shell.clone())),
                None => fail(&format!(
                    "{} is in use by another KivaDB process that does not accept connections.",
                    home.display()
                )),
            }
        }
        Err(KivaError::Locked(_)) => {
            fail(&format!("a KivaDB server or shell is already running on {}.", home.display()))
        }
        Err(e) => fail(&e.to_string()),
    };
    if args.serve {
        exit(serve(inst, &args));
    }
    if !args.no_listen {
        // Let other local shells attach to this instance.
        if let Err(e) = server::start_tcp(&inst, "127.0.0.1", 0) {
            eprintln!("[WARN] other shells will not be able to attach: {e}");
        }
    }
    on_ctrl_c(&inst, None);
    let code = shell::run_embedded(inst.clone(), args.shell.clone());
    inst.shutdown();
    exit(code);
}
