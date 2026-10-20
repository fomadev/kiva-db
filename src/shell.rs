// Copyright (c) 2026 Fordima Inc. All rights reserved.
// Licensed under the Fordima Public License (FPL) v1.0.
// See the LICENSE file in the project root for full license information.

//! The interactive shell. It either drives a local instance directly or
//! talks to a running one over the binary protocol; the experience is the same.

use crate::exec::Action;
use crate::instance::{Instance, VERSION};
use crate::lineedit::{Prompt, Sink};
use crate::proto::*;
use crate::session::Session;
use std::io::{self, BufRead, IsTerminal, Write};
use std::net::TcpStream;
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Instant;

#[derive(Default, Clone)]
pub struct ShellOpts {
    /// File where the interactive history is kept.
    pub history: Option<std::path::PathBuf>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub workspace: Option<String>,
    /// Commands given with `-e`: run them and exit.
    pub commands: Vec<String>,
    /// Connect with TLS (`--tls`).
    pub tls: bool,
    /// Expected certificate fingerprint (`--tls-fingerprint`).
    pub tls_fingerprint: Option<String>,
    /// Where trusted servers are remembered (`known_servers`).
    pub home: Option<std::path::PathBuf>,
}

/// Opens an encrypted connection, checking the server's certificate against
/// the fingerprint given or remembered; a new server is shown and, at a
/// terminal, the user is asked whether to trust it.
fn connect_secure(addr: &str, opts: &ShellOpts) -> Result<TcpStream, String> {
    let home = opts.home.clone().unwrap_or_else(crate::config::default_home);
    let known = crate::tls::known_fingerprint(&home, addr);
    let expected = opts.tls_fingerprint.clone().or(known.clone());
    let secure = crate::tls::connect(addr, expected.as_deref(), std::time::Duration::from_secs(10))
        .map_err(|e| format!("cannot connect to {addr} with TLS: {e}"))?;
    if expected.is_some() {
        if known.is_none() {
            let _ = crate::tls::remember_fingerprint(&home, addr, &secure.fingerprint);
        }
        return Ok(secure.stream);
    }
    eprintln!("The certificate of {addr} is not known yet. Its SHA-256 fingerprint is:");
    eprintln!("  {}", secure.fingerprint);
    eprintln!("Compare it with what 'kivadb tls' shows on the server.");
    if !io::stdin().is_terminal() {
        return Err(format!(
            "unknown certificate. After checking it, connect with: --tls-fingerprint {}",
            secure.fingerprint
        ));
    }
    eprint!("Trust this server and remember it? [y/N] ");
    let _ = io::stderr().flush();
    let mut answer = String::new();
    let _ = io::stdin().read_line(&mut answer);
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes" | "o" | "oui") {
        return Err("connection cancelled: the certificate was not trusted".into());
    }
    crate::tls::remember_fingerprint(&home, addr, &secure.fingerprint)
        .map_err(|e| format!("cannot remember the server: {e}"))?;
    Ok(secure.stream)
}

trait Backend {
    fn exec(&mut self, line: &str) -> (Vec<(bool, String)>, Action);
}

struct Local {
    sess: Session,
}

impl Backend for Local {
    fn exec(&mut self, line: &str) -> (Vec<(bool, String)>, Action) {
        let out = self.sess.execute(line);
        (out.lines, out.action)
    }
}

struct Remote {
    stream: TcpStream,
    rx: Receiver<(u16, Vec<u8>)>,
}

impl Backend for Remote {
    fn exec(&mut self, line: &str) -> (Vec<(bool, String)>, Action) {
        let lost = || (vec![(true, "Error: connection to the server was lost.".to_string())], Action::Exit);
        if write_frame(&mut self.stream, CMD_EXEC, line.as_bytes()).is_err() {
            return lost();
        }
        let Ok((code, payload)) = self.rx.recv() else {
            return lost();
        };
        let text = String::from_utf8_lossy(&payload);
        let lines =
            text.split('\n').filter(|l| !l.is_empty()).map(|l| (l.starts_with('-'), l[1..].to_string())).collect();
        let action = match code {
            ST_BYE => Action::Exit,
            ST_CLEAR => Action::Clear,
            _ => Action::None,
        };
        (lines, action)
    }
}

fn read_line(prompt: &str) -> Option<String> {
    if io::stdin().is_terminal() {
        print!("{prompt}");
        let _ = io::stdout().flush();
    }
    let mut line = String::new();
    match io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_string()),
    }
}

fn read_password() -> Option<String> {
    if io::stdin().is_terminal() {
        rpassword::prompt_password("Password: ").ok()
    } else {
        read_line("")
    }
}

fn show(lines: &[(bool, String)]) {
    for (err, l) in lines {
        if *err {
            eprintln!("{l}");
        } else {
            println!("{l}");
        }
    }
}

fn announce(rx: Receiver<String>, sink: Sink) {
    let _ = std::thread::Builder::new().name("kiva-shell-events".into()).spawn(move || {
        for ev in rx {
            (sink.lock().unwrap_or_else(|e| e.into_inner()))(&format!("[event] {ev}"));
        }
    });
}

fn repl(backend: &mut dyn Backend, opts: &ShellOpts, prompt: &mut Prompt) -> i32 {
    if !opts.commands.is_empty() {
        let mut code = 0;
        for cmd in &opts.commands {
            let (lines, action) = backend.exec(cmd);
            if lines.iter().any(|(e, _)| *e) {
                code = 1;
            }
            show(&lines);
            if action == Action::Exit {
                break;
            }
        }
        return code;
    }
    let interactive = io::stdin().is_terminal();
    if interactive {
        println!("KivaDB Shell v{VERSION}\nType 'help' or 'h' for command list");
    }
    while let Some(line) = prompt.read() {
        if line.trim().is_empty() {
            continue;
        }
        let start = Instant::now();
        let (lines, action) = backend.exec(&line);
        let elapsed = start.elapsed();
        show(&lines);
        match action {
            Action::Exit => break,
            Action::Clear => {
                print!("\x1b[2J\x1b[H");
                let _ = io::stdout().flush();
            }
            Action::None => {
                let first = line.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
                if interactive && !lines.is_empty() && !matches!(first.as_str(), "help" | "h" | "stats" | "scan") {
                    println!("({:.6}s)", elapsed.as_secs_f64());
                }
            }
        }
    }
    prompt.save();
    if interactive {
        println!("Goodbye.");
    }
    0
}

/// Asks for credentials up to three times. `try_login` returns the lines to
/// show and whether the attempt succeeded.
fn login(opts: &ShellOpts, mut try_login: impl FnMut(&str, &str) -> (Vec<(bool, String)>, bool)) -> bool {
    let scripted = opts.user.is_some() && opts.password.is_some();
    for _ in 0..3 {
        let Some(user) = opts.user.clone().or_else(|| read_line("Username [kiva]: ")) else {
            return false;
        };
        let Some(password) = opts.password.clone().or_else(read_password) else {
            return false;
        };
        // Just pressing Enter picks the built-in administrator.
        let user = if user.trim().is_empty() { crate::auth::DEFAULT_ADMIN } else { user.trim() };
        let (lines, ok) = try_login(user, &password);
        show(&lines);
        if ok {
            return true;
        }
        if scripted || !io::stdin().is_terminal() {
            return false;
        }
    }
    false
}

/// Shell on an instance living in this process.
pub fn run_embedded(inst: Arc<Instance>, opts: ShellOpts) -> i32 {
    let (tx, rx) = mpsc::channel();
    let mut sess = Session::new(inst.clone(), true, Some(tx));
    let (mut prompt, sink) = Prompt::new(opts.history.clone());
    announce(rx, sink);
    if inst.opts.skip_auth {
        eprintln!("[WARN] Authentication disabled - unsafe for production");
    }
    if sess.user.is_none() || opts.user.is_some() {
        let ok = login(&opts, |user, password| match sess.login(user, password) {
            Ok(u) => {
                let mut lines = vec![(false, format!("Welcome {} ({})", u.name, u.role.name()))];
                if u.has_empty_password() {
                    lines.push((
                        true,
                        format!(
                            "[WARN] {0} password is empty. Please run: modify user \"{0}\" password \"...\"",
                            u.name
                        ),
                    ));
                }
                (lines, true)
            }
            Err(e) => (vec![(true, format!("Error: {e}"))], false),
        });
        if !ok {
            eprintln!("Login failed.");
            return 1;
        }
    }
    if let Some(ws) = &opts.workspace {
        if let Err(e) = sess.switch_workspace(ws) {
            eprintln!("Error: {e}");
            return 1;
        }
    }
    let load = sess.engine.stats();
    if load.recovered_bytes > 0 {
        eprintln!(
            "[INFO] Recovered after an unclean shutdown: {} bytes of incomplete log discarded.",
            load.recovered_bytes
        );
    }
    let mut backend = Local { sess };
    repl(&mut backend, &opts, &mut prompt)
}

/// Shell connected to a running instance.
pub fn run_client(addr: &str, opts: ShellOpts) -> i32 {
    let connected = if opts.tls || opts.tls_fingerprint.is_some() {
        connect_secure(addr, &opts)
    } else {
        TcpStream::connect(addr).map_err(|e| format!("cannot connect to {addr}: {e}"))
    };
    let mut stream = match connected {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Error: {e}");
            return 1;
        }
    };
    let remote = stream.peer_addr().map(|p| !p.ip().is_loopback()).unwrap_or(false);
    if remote && !opts.tls && opts.tls_fingerprint.is_none() {
        eprintln!("[WARN] this connection is not encrypted: the password travels in clear. Add --tls.");
    }
    let _ = stream.set_nodelay(true);
    let hello = write_frame(&mut stream, CMD_HELLO, &[]).and_then(|_| read_frame(&mut stream));
    let Ok((code, hello)) = hello else {
        eprintln!("Error: {addr} did not answer like a KivaDB server.");
        return 1;
    };
    let hello = String::from_utf8_lossy(&hello).into_owned();
    if code == ST_ERR {
        eprintln!("Error: {hello}");
        return 1;
    }
    if io::stdin().is_terminal() && opts.commands.is_empty() {
        println!("Connected to {} at {addr}", hello.lines().next().unwrap_or("KivaDB"));
    }
    if hello.contains("auth=required") || opts.user.is_some() {
        let ok = login(&opts, |user, password| {
            let mut payload = user.as_bytes().to_vec();
            payload.push(0);
            payload.extend_from_slice(password.as_bytes());
            match write_frame(&mut stream, CMD_AUTH, &payload).and_then(|_| read_frame(&mut stream)) {
                Ok((code, msg)) => {
                    let text = String::from_utf8_lossy(&msg).into_owned();
                    let ok = code == ST_OK;
                    let lines = text
                        .lines()
                        .map(|l| {
                            (!ok || l.starts_with("[WARN]"), if ok { l.to_string() } else { format!("Error: {l}") })
                        })
                        .collect();
                    (lines, ok)
                }
                Err(e) => (vec![(true, format!("Error: {e}"))], false),
            }
        });
        if !ok {
            eprintln!("Login failed.");
            return 1;
        }
    }

    // One thread reads the socket: notifications are printed as they come,
    // command replies are handed to the prompt loop.
    let (tx, rx) = mpsc::channel();
    let Ok(mut reader) = stream.try_clone() else {
        eprintln!("Error: cannot set up the connection.");
        return 1;
    };
    let (mut prompt, sink) = Prompt::new(opts.history.clone());
    let _ = std::thread::Builder::new().name("kiva-client-read".into()).spawn(move || {
        while let Ok((code, payload)) = read_frame(&mut reader) {
            if code == ST_EVENT {
                let ev = String::from_utf8_lossy(&payload);
                (sink.lock().unwrap_or_else(|e| e.into_inner()))(&format!("[event] {ev}"));
            } else if tx.send((code, payload)).is_err() {
                break;
            }
        }
    });
    let mut backend = Remote { stream, rx };
    if let Some(ws) = &opts.workspace {
        let (lines, _) = backend.exec(&format!("workspace switch \"{ws}\""));
        if lines.iter().any(|(e, _)| *e) {
            show(&lines);
            return 1;
        }
    }
    repl(&mut backend, &opts, &mut prompt)
}
