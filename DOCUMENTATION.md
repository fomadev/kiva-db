# KivaDB Documentation

The complete manual for KivaDB 1.7.1. It starts from zero: if you have never used a database, begin at [Chapter 0](#chapter-0--introduction) and read in order. If you already know what you are looking for, use the table of contents or jump to the [cheat sheet](#chapter-31--command-cheat-sheet).

## Table of contents

**Part I — Getting started**

* [Chapter 0 — Introduction](#chapter-0--introduction)
* [Chapter 1 — Installation](#chapter-1--installation)
* [Chapter 2 — Starting and stopping KivaDB](#chapter-2--starting-and-stopping-kivadb)
* [Chapter 3 — First steps in the shell](#chapter-3--first-steps-in-the-shell)

**Part II — The command language**

* [Chapter 4 — Syntax rules](#chapter-4--syntax-rules)
* [Chapter 5 — Data types](#chapter-5--data-types)
* [Chapter 6 — Writing, reading and deleting](#chapter-6--writing-reading-and-deleting)
* [Chapter 7 — Expiring keys (TTL)](#chapter-7--expiring-keys-ttl)
* [Chapter 8 — Calculations and text with print](#chapter-8--calculations-and-text-with-print)
* [Chapter 9 — Listing keys and ranges](#chapter-9--listing-keys-and-ranges)
* [Chapter 10 — Documents](#chapter-10--documents)
* [Chapter 11 — Queries and indexes](#chapter-11--queries-and-indexes)
* [Chapter 12 — Vector search for AI](#chapter-12--vector-search-for-ai)
* [Chapter 13 — Transactions](#chapter-13--transactions)

**Part III — Administration**

* [Chapter 14 — Workspaces](#chapter-14--workspaces)
* [Chapter 15 — Configuration](#chapter-15--configuration)
* [Chapter 16 — Users, roles and security](#chapter-16--users-roles-and-security)
* [Chapter 17 — Maintenance and reliability](#chapter-17--maintenance-and-reliability)
* [Chapter 18 — Keeping values in memory (value cache)](#chapter-18--keeping-values-in-memory-value-cache)
* [Chapter 19 — How data is stored](#chapter-19--how-data-is-stored)

**Part IV — Connecting applications**

* [Chapter 20 — The TCP server](#chapter-20--the-tcp-server)
* [Chapter 21 — The REST API](#chapter-21--the-rest-api)
* [Chapter 22 — Change notifications](#chapter-22--change-notifications)
* [Chapter 23 — Redis compatibility](#chapter-23--redis-compatibility)
* [Chapter 24 — Connecting from your programs](#chapter-24--connecting-from-your-programs)

**Part V — Going further**

* [Chapter 25 — Replication](#chapter-25--replication)
* [Chapter 26 — Lua scripting](#chapter-26--lua-scripting)
* [Chapter 27 — Monitoring](#chapter-27--monitoring)
* [Chapter 28 — Using KivaDB inside a Rust program](#chapter-28--using-kivadb-inside-a-rust-program)

**Part VI — Production**

* [Chapter 29 — Deploying KivaDB on a server](#chapter-29--deploying-kivadb-on-a-server)

**Part VII — Reference**

* [Chapter 30 — Troubleshooting](#chapter-30--troubleshooting)
* [Chapter 31 — Command cheat sheet](#chapter-31--command-cheat-sheet)
* [Chapter 32 — Limits and scope](#chapter-32--limits-and-scope)
* [Chapter 33 — Upgrading from KivaDB 1.x to 2.0](#chapter-33--upgrading-from-kivadb-1x-to-20)
* [Glossary](#glossary)

---

## Chapter 0 — Introduction

### What is a database?

A program usually forgets everything when it stops. A **database** is the piece of software that remembers: you hand it information, it writes that information to disk in a safe way, and gives it back whenever you ask, even after the computer has been restarted.

### What is KivaDB?

KivaDB is a **key-value and document database**:

* Every piece of information is stored under a name, called a **key**. `age` is a key; `user:1` is a key.
* What is stored under a key is its **value**. A value can be simple (a number, a piece of text) or structured (a list, or a **document** with named fields such as a user profile).

```text
key        value
---------  ------------------------------------------
age        30
name       "Fordi"
user:1     {"name": "Alice", "role": "admin"}
```

You talk to KivaDB with short commands: `set` to store, `get` to read, `del` to delete, and so on.

### What makes it a real database

* **Types.** KivaDB knows whether a value is a number or a text and refuses to mix them up by accident.
* **Durability.** What you store survives a crash. Each record on disk carries a checksum, so damaged data is detected instead of being returned as if it were good.
* **It runs as a service.** You *start* KivaDB once; it then listens on a network port (2911 by default). Any number of shells, applications and scripts can work on the same data at the same time.
* **Transactions.** Several changes can be grouped so that either all of them happen or none.
* **Multi-user.** Accounts, passwords, roles and an audit log.
* **Several models in one.** Plain key-value, JSON documents with indexes and queries, and vector search for AI embeddings, all in the same keys and the same files. Applications written for Redis can connect to it too.

### What KivaDB is good for

| Use | Why it fits |
| :--- | :--- |
| Sessions, login tokens, temporary codes | Keys that expire by themselves ([Chapter 7](#chapter-7--expiring-keys-ttl)) |
| User profiles, settings, product catalogues | JSON documents, fields read and changed one by one, indexes and queries ([Chapters 10 and 11](#chapter-10--documents)) |
| Counters, quotas, rate limits | Atomic scripts and transactions ([Chapters 13](#chapter-13--transactions) and [26](#chapter-26--lua-scripting)) |
| Memory of an AI assistant, search by meaning | Vector search ([Chapter 12](#chapter-12--vector-search-for-ai)) |
| Event logs, sensor readings by time | Sorted keys make time ranges cheap ([Chapter 9](#chapter-9--listing-keys-and-ranges)) |
| Live dashboards, notifications to web pages | Change notifications over WebSocket and Server-Sent Events ([Chapter 22](#chapter-22--change-notifications)) |
| Replacing a Redis used as a cache or a session store, with data kept on disk | The Redis protocol, sets and sorted sets included ([Chapter 23](#chapter-23--redis-compatibility)) |
| Applications that call a language model | A semantic cache that answers a question already asked in other words ([12.8](#128-semantic-cache-reuse-answers-to-the-same-question)) |
| Personal, health or payment data | Encryption at rest and on the network ([16.11](#1611-encrypted-connections-tls), [16.12](#1612-encryption-at-rest)) |
| A service that must keep running | Replicas and automatic failover ([Chapter 25](#chapter-25--replication)), backups in one command ([17.7](#177-backups)) |

KivaDB is **not** a relational database: there are no tables, no joins and no SQL. Applications built around relations between many tables and complex reports are better served by PostgreSQL or MySQL, possibly next to KivaDB. [Chapter 32](#chapter-32--limits-and-scope) lists the limits honestly.

### The pieces

| Piece | What it is |
| :--- | :--- |
| `kivadb` | One executable: the database server, the interactive shell and the command-line tools |
| Kiva Studio | The desktop application to browse data, run queries and manage users ([24.7](#247-kiva-studio-the-graphical-client)) |
| Drivers | The official Node.js driver, the Rust client, and any Redis or HTTP library ([Chapter 24](#chapter-24--connecting-from-your-programs)) |

A started KivaDB listens on three ports, all on the local machine by default:

| Port | Protocol | Who uses it |
| :--- | :--- | :--- |
| **2911** | KivaDB protocol | Shells, Kiva Studio, the Node.js and Rust clients ([Chapter 20](#chapter-20--the-tcp-server)) |
| **8080** | HTTP | The REST API, web pages, metrics ([Chapter 21](#chapter-21--the-rest-api)) |
| **6379** | Redis (RESP) | Redis client libraries and `redis-cli` ([Chapter 23](#chapter-23--redis-compatibility)) |

### How this manual is organised

| Part | Chapters | Subject | Read it when |
| :--- | :--- | :--- | :--- |
| I | 0 – 3 | Getting started | You are new to KivaDB |
| II | 4 – 13 | The command language, from the simplest command to vectors and transactions | You store and read data |
| III | 14 – 19 | Administration: workspaces, configuration, users, maintenance, cache, files | You are responsible for a KivaDB |
| IV | 20 – 24 | Connecting applications: binary protocol, REST, notifications, Redis, drivers | You write a program that uses KivaDB |
| V | 25 – 28 | Replication, Lua scripts, monitoring, embedding in Rust | You need more |
| VI | 29 | Deploying on Linux, Windows, macOS and Docker | KivaDB goes on a server |
| VII | 30 – 32 | Troubleshooting, cheat sheet, limits, glossary | You look something up |

### Conventions

Every example shows what you type after the `kiva>` prompt and, below it, what KivaDB answers:

```text
kiva> get name
name: Fordi [string]
```

In the descriptions of commands:

| Notation | Meaning | Example |
| :--- | :--- | :--- |
| `<key>` | Replace with your own value | `get <key>` → `get age` |
| `[ttl <seconds>]` | Optional part | `set a 1` or `set a 1 ttl 60` |
| `add\|set` | One of the choices | `bump session add 60` |
| `...` | More of the same | `get a and b and c` |

Blocks marked `bash` are typed in an ordinary terminal (PowerShell or Command Prompt on Windows, Terminal on macOS and Linux), not in the KivaDB shell.

---

## Chapter 1 — Installation

There are three ways to get KivaDB. Pick one:

| Way | For whom | Section |
| :--- | :--- | :--- |
| Download the ready-made executable | Most people | [1.1](#11-download-the-ready-made-executable) |
| Run the Docker image | Servers and anyone who already uses Docker | [1.2](#12-run-the-docker-image) |
| Build it from the source code | Contributors, and systems without a ready-made executable | [1.3](#13-build-from-the-source-code) |

### 1.1 Download the ready-made executable

Every version of KivaDB is published on the [releases page](https://github.com/fomadev/kiva-db/releases) of the project. Download the archive for your system:

| System | Archive | Contains |
| :--- | :--- | :--- |
| Windows (64-bit, Intel or AMD) | `kivadb_win_x64.zip` | `kivadb.exe` |
| Linux (64-bit, Intel or AMD) | `kivadb_linux_x64.tar.gz` | `kivadb` |
| macOS (Apple Silicon: M1, M2, M3...) | `kivadb_mac_arm64.tar.gz` | `kivadb` |

KivaDB is **one file** with nothing else to install. Unpack the archive and put the file in a folder of your choice.

**Linux and macOS in one command.** The installation script downloads the latest release for your system, checks its SHA-256 fingerprint and installs it in `/usr/local/bin` (it asks for your password through `sudo` when that folder needs it):

```bash
curl -sSL https://raw.githubusercontent.com/fomadev/kiva-db/main/install.sh | bash
```

`KIVADB_INSTALL_DIR` chooses another folder and `KIVADB_VERSION` another version: `curl -sSL .../install.sh | KIVADB_INSTALL_DIR=$HOME/.local/bin bash`. Prefer to read a script before running it? Download it first, open it, then run `sh install.sh`. The manual steps below do the same by hand.

**Windows in one command.** In PowerShell (Start menu, type *PowerShell*), the installation script downloads the latest release, checks its SHA-256 fingerprint, puts `kivadb.exe` in `%LOCALAPPDATA%\Programs\KivaDB` (no administrator rights needed) and adds that folder to your `PATH`:

```powershell
irm https://raw.githubusercontent.com/fomadev/kiva-db/main/install.ps1 | iex
```

`irm` downloads the script and `iex` runs it. Open a new terminal afterwards so that `kivadb` is found everywhere. `$env:KIVADB_INSTALL_DIR` chooses another folder, `$env:KIVADB_VERSION` another version, `$env:KIVADB_NO_MODIFY_PATH = "1"` leaves the `PATH` alone; set them before running the line. If KivaDB is running, the script stops and asks to run `kivadb stop` first (or to use `kivadb update`). It works with the PowerShell of Windows 10 and 11 and with PowerShell 7. Prefer to read a script before running it? Download `install.ps1`, open it, then run `powershell -ExecutionPolicy Bypass -File install.ps1`.

**Windows by hand.** Right-click the zip, *Extract All*, then move `kivadb.exe` to a folder such as `C:\Tools`. Windows SmartScreen may warn about an unknown publisher the first time: choose *More info*, then *Run anyway*.

**Linux:**

```bash
tar -xzf kivadb_linux_x64.tar.gz
sudo install -m 755 kivadb /usr/local/bin/kivadb
```

**macOS:**

```bash
tar -xzf kivadb_mac_arm64.tar.gz
xattr -d com.apple.quarantine kivadb
sudo install -m 755 kivadb /usr/local/bin/kivadb
```

The `xattr` line tells macOS that you trust this file. Without it, macOS refuses to open an executable that was downloaded from the Internet and is not signed by Apple (you can also right-click it in the Finder and choose *Open* once).

**Checking the download (optional).** The release also contains `SHA256SUMS.txt`, the fingerprint of every archive. Compare it with the fingerprint of your file: `sha256sum <archive>` on Linux, `shasum -a 256 <archive>` on macOS, `certutil -hashfile <archive> SHA256` on Windows. Identical fingerprints prove the file was not damaged or altered.

Then go to [1.4](#14-make-it-available-everywhere).

### 1.2 Run the Docker image

The image is published on Docker Hub as `fomadev0/kiva-db`:

```bash
docker run -d --name kivadb -e KIVADB_ADMIN_PASSWORD=a-long-secret -p 2911:2911 -p 8080:8080 -p 6379:6379 -v kivadb-data:/data fomadev0/kiva-db:1.7.1
```

KivaDB is then running, reachable on the three ports, with `kiva` as administrator and the password you chose. Every part of that line, Compose, backups and upgrades are explained in [29.6](#296-docker). With Docker you can skip the rest of this chapter and [Chapter 2](#chapter-2--starting-and-stopping-kivadb): to open a shell, run `docker exec -it kivadb kivadb -u kiva`.

The image is built for Intel and AMD processors (`linux/amd64`), the processors of most servers. On an ARM machine, build the image yourself from the source ([29.6](#296-docker)).

### 1.3 Build from the source code

You need:

* **Rust** (stable). Install it from [rustup.rs](https://rustup.rs).
* **A C compiler.** It is used once, to compile the Lua interpreter that KivaDB embeds. On Windows the "Build Tools for Visual Studio" installed with Rust are enough; on Linux install `gcc`; on macOS install the Xcode command line tools (`xcode-select --install`).

Get the code and build:

```bash
git clone https://github.com/fomadev/kiva-db.git
cd kiva-db
cargo build --release
```

The first build downloads the dependencies and takes a few minutes. The result is one file:

| System | File |
| :--- | :--- |
| Windows | `target\release\kivadb.exe` |
| Linux, macOS | `target/release/kivadb` |

To check your build, run the test suites (optional). They cover storage, crash recovery, every command, the network protocols, the Redis protocol and replication:

```bash
cargo test
```

### 1.4 Make it available everywhere

Put the `kivadb` file into a folder that is in your `PATH`, the list of folders where the terminal looks for programs. After that you can type `kivadb` from any folder.

* **Linux, macOS:** `/usr/local/bin` is in the `PATH`; `install.sh` and the `install` lines above already put it there.
* **Windows:** `install.ps1` already did it. Otherwise, search for *Edit environment variables for your account* in the Start menu, select `Path`, *Edit*, *New*, type the folder (for example `C:\Tools`), then *OK*. Open a **new** terminal afterwards: a terminal that was already open keeps the old `PATH`.

Check that it works:

```bash
kivadb --version
```

```text
KivaDB 1.7.1
```

`kivadb --help` lists every option.

### 1.5 Where KivaDB keeps its files

KivaDB has one **home directory** that holds its configuration, its users and its data:

| System | Home directory |
| :--- | :--- |
| Windows | `%APPDATA%\KivaDB` (for example `C:\Users\you\AppData\Roaming\KivaDB`) |
| Linux, macOS | `~/.kivadb` |

It is created automatically the first time you run KivaDB. Because the home is always the same, you find your data again whatever folder you are in.

To use another location, either pass `--home <folder>` on the command line or set the `KIVADB_HOME` environment variable. This is handy for experiments:

```bash
kivadb --home ./scratch-home
```

### 1.6 Updating

**`kivadb update`** installs the latest release in place of the executable you run:

```bash
kivadb update
```

```text
Checking for a new version of KivaDB...
New version available: 1.8.0 (current: 1.7.1).
Downloading kivadb_win_x64.zip...
Checksum verified (SHA-256).
KivaDB successfully updated to version 1.8.0!
```

What it does, in order:

1. Reads the latest release on GitHub and compares its version with yours.
2. Downloads the archive for your system (the files of [1.1](#11-download-the-ready-made-executable)).
3. Compares its SHA-256 fingerprint with the release's `SHA256SUMS.txt`. A mismatch stops everything: nothing is changed.
4. Runs the new executable once (`--version`) to make sure it works on this machine.
5. Puts it in place of the old one. On Linux and macOS the swap is atomic. On Windows a running program cannot be overwritten, so the old file is renamed `kivadb.exe.old` and removed the next time KivaDB starts.

`kivadb update --check` only tells whether a new version exists. If the executable sits in a protected folder such as `/usr/local/bin`, run `sudo kivadb update`. A KivaDB that was running keeps the old version until you restart it (`kivadb stop`, `kivadb start`); `update` reminds you. Inside a Docker container, update the image instead ([29.6](#296-docker)).

**Knowing when there is a new version.** When you use `kivadb` in a terminal, it checks GitHub at most once a day, in the background, and mentions a new release when the command ends:

```text
A new version of KivaDB is available: 1.8.0 (current: 1.7.1)
Run 'kivadb update' to install the latest release.
```

The check never slows a command down and never runs for scripts, services, `kivadb serve` or containers (only when a person is at the terminal). Its result is kept in `version_check.json` in the home directory. Set `KIVADB_NO_UPDATE_CHECK=1` to turn it off.

**By hand.** Stop KivaDB, replace the `kivadb` file with the new one, start again. Either way, the data, the users and the configuration live in the home directory, not in the executable, so they are kept. KivaDB 1.7.0 writes storage format `1`; if a later version ever changes the format, its release notes will say how to convert. Since 1.6.0, the files hold expiry times in milliseconds: 1.6.0 reads the files of earlier versions and converts their expiry times in seconds, but an earlier version must not be put back on files written by 1.6.0, whose keys with a TTL would no longer expire. An earlier version does not know the unique indexes of 1.7.0: it does not check them, and it may forget them when it saves its index definitions; if you go back and forth, check `indexes` and create them again. Make a backup before upgrading if you may need to go back ([17.7](#177-backups)). To move from 1.x to 2.0, follow [Chapter 33](#chapter-33--upgrading-from-kivadb-1x-to-20).

### 1.7 Uninstalling

Stop KivaDB, delete the `kivadb` file, and delete the home directory if you also want to remove the data. Nothing else was installed: no service, no registry entry, no background program.

---

## Chapter 2 — Starting and stopping KivaDB

A database is meant to be **running**: started once, then used by whoever needs it. KivaDB gives you three commands for that.

### 2.1 Start

```bash
kivadb start
```

```text
KivaDB started (pid 21040).
  TCP port : 2911
  TLS      : on every port, certificate SHA-256 77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6
  REST API : http://127.0.0.1:8080
  Redis    : port 6379
  Home     : C:\Users\you\AppData\Roaming\KivaDB
  Log      : C:\Users\you\AppData\Roaming\KivaDB\server.log
Open a shell with 'kivadb', stop the database with 'kivadb stop'.
```

KivaDB now runs in the background. You can close the terminal; the database keeps running. It listens on three ports:

| Port | Used for |
| :--- | :--- |
| **2911** | The KivaDB protocol: shells and client programs ([Chapter 20](#chapter-20--the-tcp-server)) |
| **8080** | The REST API over HTTP ([Chapter 21](#chapter-21--the-rest-api)) |
| **6379** | The Redis protocol ([Chapter 23](#chapter-23--redis-compatibility)) |

By default these ports only accept connections from the same computer (`127.0.0.1`). Opening KivaDB to other machines is explained in [section 16.7](#167-opening-kivadb-to-the-network).

If the REST API or the Redis port cannot start (another program already uses the port), `start` says so on those lines and KivaDB runs without it. Everything the server prints goes to `server.log` in the home directory.

### 2.2 Status

```bash
kivadb status
```

```text
KivaDB is running (pid 21040).
  TCP port : 2911
  Home     : C:\Users\you\AppData\Roaming\KivaDB
```

or, when it is stopped:

```text
KivaDB is not running. Start it with: kivadb start
```

### 2.3 Stop

```bash
kivadb stop
```

```text
KivaDB is shutting down.
KivaDB stopped.
```

Stopping is clean: everything is flushed to disk first. (Even an unclean stop loses nothing, see [Chapter 17](#chapter-17--maintenance-and-reliability).) You can also type `shutdown` in a shell; it does the same thing and requires the admin role.

When authentication is on, `kivadb stop` is a client like any other and must log in as an administrator: `kivadb stop -u kiva` asks for the password.

### 2.4 Options of `start`

| Option | Meaning |
| :--- | :--- |
| `--port <n>` | KivaDB protocol port (default 2911) |
| `--http-port <n>` | REST API port (default 8080) |
| `--no-http` | Do not start the REST API |
| `--resp-port <n>` | Redis-compatible port (default 6379) |
| `--no-resp` | Do not start the Redis-compatible listener |
| `--bind <address>` | Address to listen on (default 127.0.0.1) |
| `--home <folder>` | Use another home directory |

The same values can be set permanently in the configuration file ([Chapter 15](#chapter-15--configuration)) or through environment variables ([29.2](#292-configuring-with-environment-variables)). `start` also accepts `--skip-auth` and `--lua-debug` ([16.10](#1610-emergency-access), [26.8](#268-measuring-and-debugging)).

### 2.5 Running in the foreground

`kivadb serve` runs the same server but stays attached to the terminal and prints its log there. Stop it with Ctrl+C. Use it when you want to watch what happens, or when another program (systemd, Docker, a service manager) is in charge of keeping KivaDB alive.

```bash
kivadb serve
```

```text
KivaDB 1.7.1 server
  Home         : C:\Users\you\AppData\Roaming\KivaDB
  TCP protocol : 127.0.0.1:2911 (binary and plain text)
  TLS          : on every port, certificate SHA-256 77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6
  REST API     : http://127.0.0.1:8080  (docs: /docs, metrics: /metrics, events: /ws, /v1/stream)
  Redis (RESP) : 127.0.0.1:6379  (redis-cli and Redis client libraries)
  Auth         : disabled (local use)
  Replication  : master
Press Ctrl+C to stop.
```

### 2.6 Starting KivaDB with the computer

KivaDB does not install itself as a system service; use what your system provides. The short versions are below; [Chapter 29](#chapter-29--deploying-kivadb-on-a-server) gives complete, step-by-step instructions for Linux, Windows, macOS and Docker.

**Windows** - a scheduled task that runs at logon:

```bash
schtasks /Create /TN "KivaDB" /TR "\"C:\Tools\kivadb.exe\" start" /SC ONLOGON
```

**Linux** - a systemd unit, `/etc/systemd/system/kivadb.service`:

```ini
[Unit]
Description=KivaDB
After=network.target

[Service]
User=kivadb
Environment=KIVADB_HOME=/var/lib/kivadb
ExecStart=/usr/local/bin/kivadb serve
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now kivadb
```

### 2.7 What if I open a shell without starting KivaDB?

It works. If nothing is running, the `kivadb` shell opens the data itself, and **while that shell is open** the database is reachable on port 2911 like a started one (on another free port if 2911 is taken). When you close the shell, the database stops. Use `kivadb start` when the data must stay available.

Only one process at a time opens the files; every other `kivadb` you launch connects to it. That is why two terminals always see the same data.

---

## Chapter 3 — First steps in the shell

### 3.1 Open the shell

```bash
kivadb
```

```text
KivaDB Shell v1.7.1
Type 'help' or 'h' for command list
kiva>
```

`kiva>` is the **prompt**: KivaDB is waiting for a command. If the database was started with `kivadb start`, a line such as `Connected to KivaDB 1.7.1 at 127.0.0.1:2911` is shown first.

### 3.2 Store and read a value

```text
kiva> set name "Fordi"
OK: name saved.
(0.000412s)
kiva> get name
name: Fordi [string]
(0.000108s)
```

* `set name "Fordi"` stores the text `Fordi` under the key `name`.
* `get name` reads it back. The answer shows the key, the value, and its type in brackets.
* The last line is the time the command took. It is left out of the examples from here on.

### 3.3 Several things at once

The word `and` repeats a command:

```text
kiva> set age 30 and city "Kinshasa" and active true
OK: age saved.
OK: city saved.
OK: active saved.
kiva> get name and age and ghost
name: Fordi [string]
age: 30 [number]
ghost: (nil)
```

`(nil)` means "there is no such key".

### 3.4 Change and delete

`set` only creates. To change an existing key, use `update`:

```text
kiva> set name "Other"
Error: Key 'name' already exists. Use 'update' to change it.
kiva> update age 31
OK: age updated.
kiva> del city
Deleted: city
```

This separation protects you from overwriting a value by mistake.

### 3.5 See what is stored

```text
kiva> scan

--- KivaDB Scan (workspace: default) ---
 KEY                      | TYPE    |       SIZE | MODIFIED            | STATUS
------------------------------------------------------------------------------
 active                   | boolean |    4 bytes | 2026-10-12 09:44:54 |
 age                      | number  |    2 bytes | 2026-10-12 09:44:54 |
 name                     | string  |    5 bytes | 2026-10-12 09:44:54 |
------------------------------------------------------------------------------
Total: 3 keys.
```

### 3.6 Editing, history and completion

In a terminal the shell behaves like a modern command line:

| Key | Effect |
| :--- | :--- |
| Left / Right, Home / End | Move in the line |
| Up / Down | Previous and next commands |
| Tab | Complete a command or the next keyword (`get sim` + Tab gives `get similar`) |
| Ctrl+R | Search the history |
| Ctrl+C | Abandon the line being typed |
| Ctrl+D | Leave the shell |

The history is kept between sessions in `history.txt` in the home directory. Commands that contain a password (`create user ... identified by`, `modify user ... password`, `auth`) are never written to it.

### 3.7 Help and exit

| Command | Effect |
| :--- | :--- |
| `help` or `h` | Lists every command |
| `clear` | Clears the screen |
| `exit` or `quit` | Leaves the shell (the database keeps running if it was started) |

### 3.8 Running commands without entering the shell

`-e` runs one command and exits. It can be repeated. This is how you use KivaDB from scripts and batch files:

```bash
kivadb -e "set visits 1" -e "get visits"
```

```text
OK: visits saved.
visits: 1 [number]
```

The exit code is `0` when every command succeeded and `1` when one of them reported an error.

Commands can also be fed on standard input (`kivadb < commands.txt`); the exit code is `1` if one of them failed. For files of commands, prefer the `.kva` files of [3.10](#310-command-files-kva).

### 3.9 Commands over several lines

A document can be written over several lines. As long as a `{` or a `[` is open, pressing Enter goes on to the next line, shown by ` ...>`; the command runs when the document closes:

```text
kiva> set user:1 {
 ...>   "name": "Fordi",
 ...>   "tags": ["admin",
 ...>            "dev"]
 ...> }
OK: user:1 saved.
```

Brackets inside quoted text do not count (`set note "a { b"` is complete). Ctrl+C abandons a command being typed. The history keeps the whole command, so the Up arrow brings it back in one piece.

### 3.10 Command files (.kva)

A `.kva` file holds KivaDB commands, written as in the shell: one command per line, or over several lines for documents, with comment lines (`#` or `--`) and blank lines between commands.

```text
# strings
set nameA "Fordi Malanda"
set nameB "Nathan Malanda"

# numbers
set number ageM 24

set user:1 {
  "nom": "Fordi",
  "age": 22
}

set classe:1 [
  "1er",
  "2eme",
  "3eme"
]
```

Run it with `kivadb run`:

```bash
kivadb run data.kva
```

```text
OK: nameA saved.
OK: nameB saved.
OK: ageM saved.
OK: user:1 saved.
OK: classe:1 saved.
```

* **Errors** are reported with the file and the line of the command, and the run **stops there**: the commands after it are not run. `--keep-going` runs them anyway. The exit code is `1` when a command failed.

  ```text
  data.kva:12: Error: String values like 'oops' must be quoted ("" or '').
  data.kva: stopped at line 12 (5 of 9 commands run).
  ```

* A document that is never closed is refused **before anything runs**: `data.kva:7: this command opens a { or [ that is never closed`.
* The file is read on the machine where you type the command, and runs where the shell is connected: `kivadb --connect 31.25.236.110:2911 --tls -u kiva run data.kva` fills a remote server.
* In the shell, `source "data.kva"` runs a file the same way.
* **Encoding.** A file is read as UTF-8, with or without a byte order mark, or as UTF-16 with one. A file written on Windows by PowerShell (`>`, `Out-File`, `Set-Content`) or by Notepad therefore runs as it is (since 1.7.1). Other bytes are refused before anything runs: `cannot read data.kva: it is not UTF-8 text`. `script load` reads Lua files the same way ([26](#chapter-26--lua-scripting)).
* `set` only creates keys, so a file that creates keys fails the second time it runs (`Key 'nameA' already exists`). To make a file you can run again, write `set ... if not exists` (the keys already there are kept) or start it with `del ... if exists` (they are created again with the file's values).
* Kiva Studio runs `.kva` files from its Console ([24.7](#247-kiva-studio-the-graphical-client)).

To run several commands **all or nothing**, put them between `begin` and `commit` in the file ([Chapter 13](#chapter-13--transactions)).

---

## Chapter 4 — Syntax rules

A few rules apply to every command. They exist so that KivaDB never has to guess what you meant.

### 4.1 Keys

A key is written as plain text, without quotes.

| Rule | Example |
| :--- | :--- |
| Letters, digits and most symbols are allowed | `age`, `user:1`, `price_eur`, `log-2026.10` |
| A key cannot be made of digits only | `44` is refused, `id44` is fine |
| No spaces, quotes, commas, brackets, parentheses, `*` or `?` | |
| At most 1024 bytes | |
| Keys are case-sensitive | `Name` and `name` are two keys |

A common convention is to group keys with a prefix and a colon: `user:1`, `user:2`, `order:2026:001`. KivaDB keeps keys sorted, so keys with the same prefix sit next to each other, which makes them easy to list and query.

```text
kiva> set 44 1
Error: InvalidKeyName: '44' cannot be purely numeric.
kiva> set "name" 1
Error: Key 'name' cannot use quotes. Use bare text or backticks (``).
```

**Reserved words.** Words that are part of the language (`set`, `get`, `and`, `to`, `string`, `limit`, ...) cannot be used as keys directly. Put them between backticks if you really need one:

```text
kiva> set get 1
Error: 'get' is a reserved keyword and cannot be used as a key.
kiva> set `get` 1
OK: get saved.
kiva> get `get`
get: 1 [number]
```

### 4.2 Values

How you write a value tells KivaDB its type:

| You write | Type | Notes |
| :--- | :--- | :--- |
| `"Hello"` or `'Hello'` | string | Text **must** be quoted |
| `42`, `-3.14`, `1e9` | number | Numbers **must not** be quoted |
| `true`, `false` | boolean | Not quoted |
| `null` | null | "Exists, but has no value yet" |
| `["a", "b"]` | array | A JSON list |
| `{"name": "Fordi"}` | object | A JSON document |
| `0xDEADBEEF`, `b64:SGVsbG8=` | bytes | Raw data in hexadecimal or base64 |

```text
kiva> set city Kinshasa
Error: String values like 'Kinshasa' must be quoted ("" or '').
```

There are no escape sequences inside strings. To put a double quote in a text, surround the text with single quotes, and the other way round:

```text
kiva> set quote 'He said "hello"'
OK: quote saved.
```

### 4.3 Chaining with `and`

`and` separates several operations of the same command. Each one succeeds or fails on its own:

```text
kiva> set x 1 and y oops and z 3
OK: x saved.
Error: String values like 'oops' must be quoted ("" or '').
OK: z saved.
```

Chaining works with `set`, `get`, `update`, `del`, `has` and `typeof`.

### 4.4 Type prefixes

Most data commands accept an optional type before the key. It turns the command into a check: "do this, but only if the type is what I expect".

```text
kiva> set number age 30
OK: age saved.
kiva> get string age
Error: Type mismatch. 'age' is a [number].
kiva> del string age
Error: Type mismatch. Cannot delete 'age' because it is a [number].
```

### 4.5 Upper and lower case

Command words can be written in any case (`BEGIN`, `begin`). Keys and values are kept exactly as typed.

### 4.6 Comments

A line that starts with `#` or `--` is ignored. Useful in command files.

---

## Chapter 5 — Data types

KivaDB has ten types: string, number, boolean, null, array, object, bytes, and date, datetime and time ([5.8](#58-date-datetime-time)). `typeof` tells you the type of a key:

```text
kiva> typeof age
 -> age is a [number]
```

### 5.1 string

Text, in UTF-8. Always quoted.

```text
kiva> set greeting "Mbote na yo"
OK: greeting saved.
```

### 5.2 number

Integers and decimals, optionally with an exponent. KivaDB stores the number exactly as you wrote it.

```text
kiva> set price 19.99 and big 1e9 and debt -250
OK: price saved.
OK: big saved.
OK: debt saved.
```

### 5.3 boolean

`true` or `false`.

### 5.4 null

A key can exist without having a value yet. That is different from a key that does not exist:

```text
kiva> set pending null
OK: pending saved.
kiva> get pending and ghost
pending: null [null]
ghost: (nil)
kiva> has pending and has ghost
Yes: Key 'pending' exists.
No: Key 'ghost' does not exist.
```

A `null` key has no type yet, so its first `update` may give it a value of any type:

```text
kiva> update pending 12
OK: pending updated.
kiva> get pending
pending: 12 [number]
```

### 5.5 array

An ordered list, written as JSON. Elements can be of any type, including other arrays and objects.

```text
kiva> set colors ["red", "green", "blue"]
OK: colors saved.
kiva> get colors and colors[0]
colors: ["red","green","blue"] [array]
colors[0]: red [string]
```

Positions start at 0. `colors.0` means the same as `colors[0]`.

### 5.6 object

A document: named fields, written as JSON. Field names are quoted, as JSON requires.

```text
kiva> set user:1 {"name": "Alice", "role": "admin", "age": 30, "address": {"city": "Kinshasa"}}
OK: user:1 saved.
kiva> get user:1.address.city
user:1.address.city: Kinshasa [string]
```

Arrays and objects are covered in depth in [Chapter 10](#chapter-10--documents).

### 5.7 bytes

Raw binary data: a small image, a certificate, anything that is not text.

```text
kiva> set logo 0x89504E47
OK: logo saved.
kiva> set word b64:SGVsbG8=
OK: word saved.
kiva> set bytes note "Hello"
OK: note saved.
kiva> get logo and word and note
logo: 0x89504e47 [bytes]
word: 0x48656c6c6f [bytes]
note: 0x48656c6c6f [bytes]
```

Three ways to write bytes: hexadecimal after `0x`, base64 after `b64:`, or the `bytes` prefix followed by a quoted text. The shell always displays bytes in hexadecimal (only the first 64 bytes of large values).

### 5.8 date, datetime, time

Three types hold moments:

| Type | Written | Holds | Time zone |
| :--- | :--- | :--- | :--- |
| `datetime` | `"2026-10-27 14:30:00"` | An instant, kept in UTC to the millisecond | Shown in yours, read in yours when the text has no zone |
| `date` | `"2026-10-27"` | A day of the calendar | None: a birthday is the same day everywhere |
| `time` | `"14:30"` or `"14:30:00"` | A time of day | None: "opens at 8:00" stays 8:00 |

```text
kiva> set datetime created_at "2026-10-27 14:30:00"
OK: created_at saved.
kiva> set date born '2004-05-09'
OK: born saved.
kiva> set time opens "08:00"
OK: opens saved.
kiva> get created_at and born and opens
created_at: 2026-10-27 14:30:00 +01:00 [datetime]
born: 2004-05-09 [date]
opens: 08:00:00 [time]
```

* **The type is written** before the key, as for `number`, and the text is quoted (double or single quotes). Without it, `set ref "2026-10-27"` stores text: KivaDB never turns a text into a date by itself, so a reference that looks like a date stays what it is.
* **A datetime** may use `T` instead of the space, leave out the seconds, and carry its zone: `"2026-10-27T13:30:00Z"`, `"2026-10-27 14:30:00+01:00"`, `"2026-10-27 13:30 UTC"`. Without a zone, the text is a wall clock of your time zone ([5.9](#59-time-zones)); a date alone is midnight there.
* **The moment of the action**: `now()` (or `current_timestamp`) is the date and time to the millisecond, `today()` (or `current_date`) the day in your time zone, `current_time` the time of day:

  ```text
  kiva> set created_at now()
  kiva> set due today() + 30 days
  kiva> set reminder now() + 2 hours - 30 minutes
  kiva> set closes time "18:00" - 15 minutes
  ```

  A moment moves by `ms`, `seconds`, `minutes`, `hours`, `days` or `weeks` (singular or plural; `s`, `min`, `h`, `d`, `w` too). Months and years vary in length, so they are not units: write the date. A `date` moves by days or weeks only.
* **Parts of a moment**: `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()` and `weekday()` (1 for Monday to 7 for Sunday) are functions, not types. They work in `print` (`print month(created_at)`), in conditions and in `group by` ([11.1](#111-the-condition)). A datetime is read in your time zone: 23:30 UTC on 31 December is already 2027 in Kinshasa.
* **In documents**, JSON has no date type: dates are text in ISO 8601. `json set order:1 .created_at now()` writes `"2026-10-27T13:30:00.250Z"` (UTC). Conditions compare such texts as moments when the value is one (`now() - 7 days`, `datetime "..."`), whatever their form: with or without `Z`, milliseconds or a zone.
* **Programs** (REST, structured requests, Redis, the drivers) always get a datetime in ISO 8601, in UTC: `2026-10-27T13:30:00.000Z`. Each program shows it in the time zone of its own user. They write one as such a text, or as milliseconds since 1970.

### 5.9 Time zones

A datetime is stored in UTC, so the same instant is right wherever the server is. The time zone only decides how dates are shown and how a text without a zone is read:

```text
kiva> config set time.zone Africa/Kinshasa
kiva> timezone
Time zone: Africa/Kinshasa (from the server's time.zone). It is 2026-10-27 14:30:00 +01:00 here, 2026-10-27T13:30:00.000Z UTC.
kiva> timezone America/New_York
kiva> get created_at
created_at: 2026-10-27 09:30:00 -04:00 [datetime]
kiva> timezone default
```

* `time.zone` is the server's zone: `local` (the machine's, the default), `UTC`, a name of the IANA list such as `Africa/Kinshasa`, `Africa/Lubumbashi`, `Europe/Paris`, `America/New_York`, or an offset such as `+01:00`. Names follow summer time; offsets do not. It applies at once, also to the dates of `users` and `stats`.
* `modify user "app" timezone Africa/Lubumbashi` gives an account its own zone, from its next login (`... timezone none` to go back to the server's). Anyone may set their own; an admin, anyone's.
* `timezone <name>` changes it for the session only; `timezone default` goes back to the account's, else the server's.
* A server in another country is not "late": its clock is right, only its zone differs, and `time.zone` settles that. A clock that is really wrong is set by the system's time synchronization (NTP: `timedatectl set-ntp true` on Linux, automatic on Windows and macOS), not by KivaDB. `stats` shows the server's time in UTC and in its zone, and a replica warns when its clock is more than a second away from its master's ([25.4](#254-watching-it)).

### 5.10 Types are enforced

`update` never changes the type of a key:

```text
kiva> update age "thirty"
Error: Cannot update [number] with a value formatted as [string].
```

When you really want to change the type, say so explicitly with `change` ([section 6.6](#66-change--rename-or-migrate)).

---

## Chapter 6 — Writing, reading and deleting

### 6.1 `set` — create a key

```text
set [type] <key> <value> [ttl <seconds>] [if not exists]
```

Creates a new key. Fails if the key already exists.

```text
kiva> set login_retry 0
OK: login_retry saved.
kiva> set string motto "Kitoko"
OK: motto saved.
kiva> set session "abc123" ttl 3600
OK: session saved.
```

`ttl` makes the key disappear by itself after that many seconds ([Chapter 7](#chapter-7--expiring-keys-ttl)).

`if not exists` (in any case, at the end of the command) creates the key only when it is absent. When it exists, the command leaves it untouched and succeeds:

```text
kiva> set login_retry 0 if not exists
OK: login_retry skipped (already exists)
kiva> set counter 0 ttl 60 if not exists
OK: counter saved.
```

It applies to each part of an `and` chain separately: `set a 1 if not exists and b 2` skips `a` if it exists but still fails on `b`. Inside a transaction, the check sees the transaction's own changes.

### 6.2 `get` — read

```text
get [type] <key>
```

```text
kiva> get motto
motto: Kitoko [string]
kiva> get number login_retry
login_retry: 0 [number]
kiva> get ghost
ghost: (nil)
```

A key followed by a path reads one part of a document: `get user:1.name`, `get colors[1]`.

### 6.3 `update` — replace the value

```text
update [type] <key> <value>
```

Replaces the value of an existing key. The new value must have the same type. If the key has a TTL, it keeps it.

```text
kiva> update login_retry 5
OK: login_retry updated.
kiva> update ghost 1
Error: Key 'ghost' not found. Use 'set' to create it.
```

### 6.4 `has` and `typeof` — ask without reading

Both answer from memory, without touching the disk.

```text
kiva> has motto
Yes: Key 'motto' exists.
kiva> has number motto
No: Key 'motto' exists but type is [string] (expected [number]).
kiva> typeof motto and ghost
 -> motto is a [string]
 -> ghost does not exist (nil)
```

### 6.5 `del` — delete

```text
del [type] <key> [if exists]
del all keys
```

```text
kiva> del motto and ghost
Deleted: motto
Not found: ghost
```

Deleting a missing key is never an error. `if exists` (in any case, at the end) says so in a neutral line, which reads better in scripts that are run again:

```text
kiva> del motto if exists
OK: motto skipped (does not exist)
```

A type given with `del` is still checked: `del string age if exists` fails when `age` is a number.

`del all keys` empties the current workspace completely. It needs the admin role and cannot be undone:

```text
kiva> del all keys
All keys deleted. Database reset.
```

### 6.6 `change` — rename or migrate

```text
change [type] <old> to [type] <new> [value]
```

`change` does two jobs.

**Rename** a key (the value, type and TTL are kept):

```text
kiva> set active_id 7
OK: active_id saved.
kiva> change active_id to session_id
Renamed: active_id -> session_id (Type preserved)
```

**Migrate** a key to a new value, possibly of another type:

```text
kiva> set legacy_code "old"
OK: legacy_code saved.
kiva> change string legacy_code to number system_code 1044
Migrated: legacy_code -> system_code (Type: number)
```

Both forms are atomic: the old key disappears only if the new one was written. A failed migration leaves everything as it was:

```text
kiva> change system_code to boolean flag maybe
Error: Migration failed ('maybe' is not a valid boolean (use true or false).) Source key 'system_code' has been preserved.
```

`change` refuses to overwrite an existing key.

### 6.7 `add` — let KivaDB name the key

```text
add <prefix> <value> [ttl <seconds>]
```

Creates a key named `<prefix>` followed by a new identifier, and says which key it is. Use it when the application has no natural name for a record (an order, a message, a place) and should not have to invent one:

```text
kiva> add place: {"type": "VILLE", "name": "Bukavu"}
OK: place:6adf9c3ca32f23553b1dbb10 saved.
kiva> add place: {"type": "VILLE", "name": "Uvira"}
OK: place:6adf9c3ca32f23553b1dbb11 saved.
kiva> add session: "abc123" ttl 3600
OK: session:6adf9c3ca32f23553b1dbb12 saved.
```

The identifier has the format of MongoDB's ObjectId: 24 hexadecimal characters, made of the time in seconds, a random part drawn once per server process and a counter. So:

* identifiers made later sort after the earlier ones: `keys place:` lists the places in the order they were added (to the second, and exactly within one server);
* two servers do not make the same identifier, and a data set migrated from MongoDB keeps its identifiers in the same format;
* the first 8 characters give the creation time: `6adf9c3c` is 1 793 039 420 seconds after 1970 (26 October 2026, 18:30:20 UTC).

The prefix is bare text, usually ending with `:`. The value follows the rules of `set`, without a type keyword. Inside a transaction the key is chosen at once and written at `commit`. Unique indexes ([11.7](#117-unique-indexes)) apply to added documents as to the others. The structured request `add` does the same from programs and answers the key and the identifier ([20.4](#204-structured-requests-for-tools)).

---

## Chapter 7 — Expiring keys (TTL)

TTL means "time to live". A key with a TTL deletes itself when the time is up. Typical uses: login sessions, temporary codes, cached results.

### 7.1 Setting a TTL

```text
kiva> set session "abc123" ttl 3600
OK: session saved.
```

`session` will exist for 3600 seconds (one hour). `scan` shows the remaining time in the STATUS column: `TTL: 3600s`.

### 7.2 `bump` — extend or reset

```text
bump [type] <key> add|set <seconds>
```

* `add` adds time to what is left.
* `set` restarts the countdown from now. This is the classic "sliding session": every time the user does something, give them a fresh hour.

```text
kiva> bump session add 600
OK: Key 'session' TTL updated successfully (add 600s).
kiva> bump session set 3600
OK: Key 'session' TTL updated successfully (set 3600s).
kiva> bump ghost set 10
Error: Key 'ghost' not found or already expired.
```

`bump` also works on a key that had no TTL: it gives it one.

### 7.3 What happens at expiry

Once the time has passed, the key behaves exactly as if it had been deleted: `get` answers `(nil)`, `has` answers no, and it no longer appears in listings. `update` keeps the TTL of a key; `change` keeps it when renaming.

Expired keys also leave memory by themselves: every second, KivaDB removes those whose time has passed. It keeps the keys with a TTL sorted by deadline, so this costs nothing for the keys that have not expired, however many there are. When very many keys expire at the same moment, they are removed by batches of 10 000, one right after the other, so that other commands never wait long. `stats` shows how many keys have a TTL (`Total Keys : 120 (8 with TTL)`).

---

## Chapter 8 — Calculations and text with print

`print` evaluates expressions and displays the result. It does not store anything.

### 8.1 Arithmetic

Operators: `+`, `-`, `*`, `/`, `%` (remainder), with parentheses.

```text
kiva> print (4 + 8) * 10
120
kiva> print 7 / 2, 7 % 2
3.5 1
```

### 8.2 Using stored values

Write a key name and `print` uses its value:

```text
kiva> set age 31
OK: age saved.
kiva> print (age + 9) * 2, "years"
80 years
```

Several expressions separated by commas are displayed on one line, separated by spaces. Paths into documents work too: `print user:1.age + 1`.

The parts of a date, a time or a datetime are functions ([5.8](#58-date-datetime-time)): `print year(created_at), month(created_at), weekday(born)` gives `2026 10 7`. `print now(), today()` shows the moment in your time zone.

### 8.3 Text and interpolation

Inside a quoted text, `${...}` is replaced by the result of the expression between the braces:

```text
kiva> set name "Fordi"
OK: name saved.
kiva> print "Hello ${name}, next year you will be ${age + 1}"
Hello Fordi, next year you will be 32
```

`+` joins two texts:

```text
kiva> print "Kiva" + "DB"
KivaDB
```

### 8.4 Errors

`print` never guesses:

```text
kiva> print "a" + 1
Error: TypeError: cannot concatenate str and int
kiva> print 1 / 0
Error: ZeroDivisionError: division by zero
kiva> print ghost
Error: NameError: 'ghost' is not defined
kiva> print 1 2
Error: SyntaxError: missing separator between tokens.
```

A key whose name contains a character that looks like an operator (`-` for example) must be written between backticks in an expression: ``print `log-count` + 1``.

---

## Chapter 9 — Listing keys and ranges

KivaDB keeps all keys **sorted** in memory. Listing a slice of them is therefore very cheap. The examples below use these keys:

```text
kiva> set user:1 {"name": "Alice", "role": "admin", "age": 30, "address": {"city": "Kinshasa"}}
kiva> set user:2 {"name": "Bob", "role": "dev", "age": 25, "address": {"city": "Goma"}}
kiva> set user:3 {"name": "Carol", "role": "dev", "age": 41, "address": {"city": "Kinshasa"}}
kiva> set active true and age 31 and colors ["red", "green", "blue"]
```

### 9.1 `keys` — names only

```text
keys [like <pattern>] [from <A> to <B>] [limit N] [reverse]
```

```text
kiva> keys
active
age
colors
user:1
user:2
user:3
(6 keys)
kiva> keys like "user:*" limit 2
user:1
user:2
(2 keys)
kiva> keys from "b" to "v" reverse
user:3
user:2
user:1
colors
(4 keys)
```

In a pattern, `*` stands for any run of characters and `?` for exactly one character.

### 9.2 `scan` — names with details

```text
scan [pattern] [limit N]
```

Shows, for each key: type, size, last modification and status (remaining TTL, compression, MessagePack encoding).

```text
kiva> scan "user:*"

--- KivaDB Scan (workspace: default) ---
 KEY                      | TYPE    |       SIZE | MODIFIED            | STATUS
------------------------------------------------------------------------------
 user:1                   | object  |   70 bytes | 2026-10-12 09:44:54 |
 user:2                   | object  |   62 bytes | 2026-10-12 09:44:54 |
 user:3                   | object  |   68 bytes | 2026-10-12 09:44:54 |
------------------------------------------------------------------------------
Total: 3 keys.
```

### 9.3 `range` — keys and values between two bounds

```text
range <from> to <to> [reverse] [limit N] [return field1, field2]
```

Both bounds are included. `get range ...` is accepted as a synonym.

```text
kiva> range "user:1" to "user:2"
user:1: {"name":"Alice","role":"admin","age":30,"address":{"city":"Kinshasa"}} [object]
user:2: {"name":"Bob","role":"dev","age":25,"address":{"city":"Goma"}} [object]
(2 keys)
```

`return` keeps only some fields of each document (a **projection**):

```text
kiva> range "user:1" to "user:3" return name, role
user:1: {"name":"Alice","role":"admin"}
user:2: {"name":"Bob","role":"dev"}
user:3: {"name":"Carol","role":"dev"}
(3 keys)
```

`reverse` walks from the highest key down, and `limit` stops after N results. Together they answer "the last N":

```text
kiva> range "user:1" to "user:9" reverse limit 1 return name
user:3: {"name":"Carol"}
(1 keys)
```

### 9.4 Why ranges matter

Because keys are sorted as text, well-chosen key names give you useful queries for free:

* **Pagination**: `keys from "user:" to "user:~" limit 20`, then start again from the last key you received.
* **Time slices**: with keys like `log:2026-10-12T09:44`, `range "log:2026-10-12T09" to "log:2026-10-12T10"` returns one hour of logs.
* **Groups**: everything starting with `sensor:` is `keys like "sensor:*"`.

Keys are compared character by character, so numbers inside keys should be padded with zeros: `order:007` sorts before `order:010`, but `order:7` sorts after `order:10`.

---

## Chapter 10 — Documents

Arrays and objects are stored as real structures, not as text. You can read and modify a single field without rewriting the rest yourself.

### 10.1 Paths

A **path** designates a place inside a document:

| Path | Meaning |
| :--- | :--- |
| `.name` | the field `name` |
| `.address.city` | the field `city` inside the field `address` |
| `.tags[0]` or `.tags.0` | the first element of the array `tags` |
| `.tags[-1]` | the last element of `tags` (`-2` the one before, and so on) |
| `["read.time"]` | the field `read.time`: a name with a dot, a space or a bracket goes between quotes and brackets |

In the shell, quote a path that contains quotes, with the other kind of quote: `'.meta["read.time"]'`.

### 10.2 Reading a field

Directly with `get`:

```text
kiva> get user:1.name and user:1.address.city
user:1.name: Alice [string]
user:1.address.city: Kinshasa [string]
kiva> get user:1.missing
user:1.missing: (nil)
```

Or with `json get`, which takes the key and the path separately:

```text
kiva> json get user:1 .address
user:1.address: {"city":"Kinshasa"} [object]
```

If a key's name itself contains a dot (`config.v2`) and exists, `get config.v2` reads that key; the path interpretation is only tried when no such key exists.

### 10.3 Changing a field

```text
json set <key> <.path> <value>
```

```text
kiva> json set user:2 .role "lead"
OK: user:2.role updated.
kiva> json set user:2 .address.zip 243
OK: user:2.address.zip updated.
kiva> get user:2
user:2: {"name":"Bob","role":"lead","age":25,"address":{"city":"Goma","zip":243}} [object]
```

Missing intermediate fields are created. In an array, you can replace an existing position or append at the position just after the last one.

### 10.4 Removing a field

```text
kiva> json delete user:2 .age
Deleted: user:2.age
kiva> json delete user:2 .nothing
Not found: user:2.nothing
```

Every change of a field (`json set`, `json delete`, and those of [10.8](#108-arrays-and-numbers-in-place)) reads the document, changes it and writes it back as one step: two clients changing different fields of the same document at the same moment both see their change kept. Inside a transaction, the change waits for `commit` like any other write.

### 10.5 Replacing the whole document

`update` with a new JSON literal replaces everything:

```text
kiva> update colors ["black", "white"]
OK: colors updated.
```

### 10.6 JSON text stored as a string

If you store JSON inside quotes, it is a string, not a document:

```text
kiva> set raw '{"name": "Fordi"}'
OK: raw saved.
kiva> typeof raw
 -> raw is a [string]
```

Reading with a path still works on such strings (`get raw.name`), but `json set`, indexes and `where` queries need a real document. To store quoted JSON as a document, add the type:

```text
kiva> set object parsed '{"name": "Fordi"}'
OK: parsed saved.
```

### 10.7 MessagePack encoding

Documents are written to disk as JSON text by default. MessagePack is a binary encoding of the same data: smaller and faster to decode. You choose per key, and nothing else changes; every command behaves the same.

```text
kiva> convert user:1 to msgpack
OK: user:1 converted to MessagePack (70 -> 51 bytes).
kiva> get user:1.name
user:1.name: Alice [string]
kiva> convert user:1 to json
OK: user:1 converted to JSON (51 -> 70 bytes).
```

`scan` shows `msgpack` in the STATUS column for keys stored that way.

### 10.8 Arrays and numbers in place

```text
json append <key> <.path> <value> [, <value>...]
json insert <key> <.path[n]> <value>
json incr <key> <.path> <number>
```

```text
kiva> set post:1 {"title": "Hello", "tags": ["news", "tech"], "views": 0}
OK: post:1 saved.
kiva> json append post:1 .tags "africa"
OK: post:1.tags now has 3 elements.
kiva> json insert post:1 .tags[0] "featured"
OK: inserted at post:1.tags[0]; the array now has 4 elements.
kiva> json get post:1 .tags[-1]
post:1.tags[-1]: africa [string]
kiva> json delete post:1 .tags[-1]
Deleted: post:1.tags[-1]
kiva> json incr post:1 .views 1
post:1.views: 1 [number]
kiva> json incr post:1 .stats.shares 1
post:1.stats.shares: 1 [number]
```

* `json append` adds at the end; a missing field becomes a new array.
* `json insert` puts the value before the position given: `[0]` at the start, `[-1]` before the last element, the array's length at the end.
* `json incr` adds a number, negative to subtract; a missing field counts as 0. Whole numbers stay whole while they fit in 64 bits, and a sum that would not fit is refused.

Because each of these is one step on the server ([10.4](#104-removing-a-field)), a counter kept with `json incr` by many clients at once never loses a step.

### 10.9 Inspecting a document

```text
kiva> json type post:1 .tags
post:1.tags: [array]
kiva> json len post:1 .tags
post:1.tags: 3
kiva> json keys post:1
title
tags
views
stats
```

`json len` counts the elements of an array, the fields of an object or the characters of a string. Without a path, `json get`, `json type`, `json len` and `json keys` look at the whole document.

---

## Chapter 11 — Queries and indexes

So far every command named the keys it wanted. A **query** finds documents by what they contain. The examples of this chapter use the three users exactly as they were created at the start of [Chapter 9](#chapter-9--listing-keys-and-ranges).

### 11.1 The condition

```text
<pattern>.<field> <operator> <value>
```

* `pattern` selects keys (`user:*`, or `*` for all).
* `field` is a path inside the document (`role`, `address.city`).
* `operator` is one of `=`, `!=`, `>`, `>=`, `<`, `<=`, `in`, `like`, `ilike`.
* `value` follows the usual rules: text quoted, numbers and booleans bare.

| Operator | Value | Keeps the documents whose field |
| :--- | :--- | :--- |
| `=` `!=` `>` `>=` `<` `<=` | one value | compares so with the value |
| `in` | a list: `["Goma", "Bukavu"]` | equals one of the values (at most 1 024) |
| `like` | a quoted text pattern | is text matching the pattern: `%` stands for any text (none included), `_` for one character, `\%` and `\_` for the characters themselves |
| `ilike` | a quoted text pattern | is text matching the pattern, capitals and small letters being the same in every alphabet (`"KIN%"` matches `Kinshasa`, `"é%"` matches `Élisabethville`) |

`not` written before `in`, `like` or `ilike` negates it: `role not in ["admin", "ops"]`, `name not like "%test%"`. `ilike` without a wildcard compares text without regard to capitals: `name ilike "kinshasa"`.

```text
kiva> get where user:*.address.city in ["Goma", "Bukavu"]
user:2: {"name":"Bob","role":"dev","age":25,"address":{"city":"Goma"}}
(1 matches)
kiva> get where user:*.role not in ["admin", "ops"] return name
user:2: {"name":"Bob"}
user:3: {"name":"Carol"}
(2 matches)
kiva> get where user:*.name like "C%"
user:3: {"name":"Carol","role":"dev","age":41,"address":{"city":"Kinshasa"}}
(1 matches)
kiva> count where user:*.address.city ilike "kinshasa"
Count: 2
```

Regular expressions are not part of the language: `like` and `ilike` cover prefixes, suffixes and words inside a text, and an index can serve them ([11.4](#114-indexes--making-queries-fast)).

**Dates.** The value may be a moment ([5.8](#58-date-datetime-time)): `now()`, `today()`, `datetime "2026-10-27 14:30"`, `time "08:00"`, moved by durations. The field, ISO text in the documents, is then compared as a moment, whatever its form. A function compares a part of the field's date:

```text
kiva> get where order:*.created_at > now() - 7 days
kiva> get where order:*.created_at >= datetime "2026-10-01" and created_at < datetime "2026-11-01"
kiva> get where year(order:*.created_at) = 2026 and month(created_at) = 10
kiva> get where weekday(order:*.created_at) in [6, 7]
kiva> get where place:*.opens <= time "08:00"
kiva> count order:* group by month(order:*.created_at)
```

* `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()` and `weekday()` read the field in your time zone. A document whose field is not a date does not match.
* A date compared with `today()` is text compared with text: `"2026-10-27"` sorts as it should.
* A B-tree index on the field answers comparisons with a moment, and `year(...)` too: `year(x) = 2026` runs as `x >= datetime "2026-01-01" and x < datetime "2027-01-01"`, in your time zone (`explain` shows it). The other functions read every document of the pattern.

The field starts at the first dot after the last `*`: in `user:*.address.city`, the pattern is `user:*` and the field is `address.city`. Spaces around the operator are optional (`user:*.age>26`). If the pattern itself needs quoting, quote the whole left side: `"*.active" = true`.

**Several comparisons** are joined with `and`, `or` and `not`, grouped with parentheses. The first comparison names the pattern; the next ones name the field alone, or repeat the pattern:

```text
user:*.age > 26 and (role = "dev" or role = "admin")
user:*.role = "dev" and not address.city = "Goma"
```

* `not` applies first, then `and`, then `or`: `a or b and c` means `a or (b and c)`. Use parentheses when in doubt.
* A comparison on a field the document lacks is false, so `not` of it is true: `not user:*.city = "Goma"` also keeps the users without a city.
* Every comparison tests a field of the same documents: `user:*.age > 20 and order:*.total > 5` is refused (that would be a join, [11.6](#116-what-a-query-does-not-do)).
* A query holds at most 64 comparisons, nested at most 16 levels deep.

### 11.2 `get where` — find documents

```text
get where <condition> [order by <field> [asc|desc]] [limit N] [after <key>] [reverse] [return field1, field2]
```

The condition is one comparison or several ([11.1](#111-the-condition)).

```text
kiva> get where user:*.address.city = "Kinshasa"
user:1: {"name":"Alice","role":"admin","age":30,"address":{"city":"Kinshasa"}}
user:3: {"name":"Carol","role":"dev","age":41,"address":{"city":"Kinshasa"}}
(2 matches)
kiva> get where user:*.age < 35 return name, address.city
user:1: {"name":"Alice","address.city":"Kinshasa"}
user:2: {"name":"Bob","address.city":"Goma"}
(2 matches)
```

```text
kiva> get where user:*.age > 26 and (role = "admin" or address.city = "Goma")
user:1: {"name":"Alice","role":"admin","age":30,"address":{"city":"Kinshasa"}}
(1 matches)
```

Comparisons are made between values of the same kind: numbers with numbers, text with text (alphabetical order). A document that lacks the field never matches that comparison.

#### Order and pages

Rows come in key order; `reverse` starts from the last key. `order by <field>` sorts them by the value of a field instead, `asc` (the default) or `desc`. Rows of the same value come in key order.

```text
kiva> get where user:*.age > 20 order by age desc limit 2 return name, age
user:3: {"name":"Carol","age":41}
user:1: {"name":"Alice","age":30}
(2 matches)
kiva> get where user:*.age > 20 order by age desc limit 2 after user:1 return name, age
user:2: {"name":"Bob","age":25}
(1 matches)
```

`after <key>` gives the next page: the rows that come after that key, in the same order. Pass the last key of the page you have. Rows written or deleted meanwhile before that point do not shift the page: no row comes twice, none is skipped. In key order the key may have been deleted since. In the order of a field it must still be a document, since its value says where the next page starts; otherwise the error says so.

How values sort:

* By kind first: `false` and `true`, then numbers, then texts; then by value.
* A datetime with an offset (`2026-10-27T15:30:00+01:00`, `...Z`) sorts by its instant: `15:30+01:00` comes before `15:00Z`, and fractions of a second count. A datetime without an offset is a wall clock, whose instant depends on the zone, and sorts as its text. A time `08:00` sorts as `08:00:00`. A date `2026-10-27` sorts as its text, which is its order.
* Documents without the field, or holding `null`, an array or an object there, come last in both directions.

With a B-tree index on the field and a `limit`, KivaDB reads the index from its first value (its last, with `desc`). It stops once it has `limit` rows that pass the condition, reading only their documents: `INDEX ORDER SCAN` in `explain` ([11.5](#115-explain--see-how-a-query-will-run)). Otherwise the documents found are sorted in memory: only the first `limit` are kept while they are read, or all of them without a `limit`. A query sorts by one field, and the aggregates do not sort.

### 11.3 Aggregates: `count`, `sum`, `avg`, `min`, `max`, `group by`

```text
count where <condition>                 sum|avg|min|max where <condition>
count <pattern>[.<field>] [where <condition>]
sum|avg|min|max <pattern>.<field> [where <condition>]
                                        ... [group by <field>] [limit N]
```

```text
kiva> count where user:*.age > 26
Count: 2
kiva> sum where user:*.age > 0
Sum: 96 (3 matches)
kiva> avg user:*.age where role = "dev"
Avg: 33 (2 values, 2 matches)
kiva> max user:*.name
Max: "Carol" (3 values, 3 matches)
kiva> count user:* group by address.city
Count by address.city (2 groups, 3 matches):
  "Goma": 1
  "Kinshasa": 2
kiva> avg user:*.age group by role
Avg of age by role (2 groups, 3 matches):
  "admin": 30 (1 values)
  "dev": 33 (2 values)
```

* **What is read.** After `where`, `sum`, `avg`, `min` and `max` read the field the condition tests; when it tests several fields, name the one to read first: `avg user:*.age where role = "dev"`. Without `where`, every document of the pattern counts.
* **Values.** `sum` and `avg` take the numbers and skip the other values (`"n/a"`, a missing field); `avg` says how many it took. `min` and `max` compare numbers with numbers and text alphabetically, and put text after numbers. With no value, `avg`, `min` and `max` give `null`. `count <pattern>.<field>` counts the documents that have the field.
* **`group by <field>`** gives one line per distinct value of the field, in order: `null` (documents without it) first, then booleans, numbers, text. One aggregation builds at most 100 000 groups; group by a field with fewer distinct values.
* **`limit N`** stops after N matching documents, before grouping.
* **Cost.** An aggregate reads the documents its condition selects, once, through an index when it can ([11.4](#114-indexes--making-queries-fast)), and keeps only the running totals: no document is held in memory. `explain avg ...` shows the plan.

From programs, the structured request `aggregate` computes several aggregates at once ([20.4](#204-structured-requests-for-tools)).

### 11.4 Indexes — making queries fast

Without help, a query has to open every document that matches the pattern. A **secondary index** keeps, in memory, the list of keys for each value of a field, so the answer is found directly.

```text
create index on <pattern> (<field>) [using btree|hash]
create index on <pattern>.<field> [using btree|hash]
```

```text
kiva> create index on user: (role)
Index created on user:* (role): 3 documents indexed.
```

A pattern without wildcard is taken as a prefix: `user:` means `user:*`. The second form splits pattern and field as conditions do ([11.1](#111-the-condition)): `create index on user:*.address.city` indexes `address.city`.

An index keeps its values one of two ways:

| Kind | Answers | Use it for |
| :--- | :--- | :--- |
| `btree` (the default) | `=`, `>`, `>=`, `<`, `<=`, `in`, `like`, `ilike` | Numbers, dates, names, anything compared or sorted |
| `hash` | `=` and `in`, in the same time whatever the number of distinct values; `like`, `ilike` | Identifiers, e-mails, codes: fields only ever compared with `=` |

For `in`, the index looks up each value of the list. For `like` with text before its first wildcard (`"Kin%"`), a B-tree reads only the values that start with that text. Otherwise (`ilike`, `"%sha"`), the index tests each of its distinct values against the pattern, without opening a document: much faster than reading the documents when many of them share few values.

```text
kiva> create index on user:*.name using hash
Index created on user:* (name) using hash: 3 documents indexed.
```

A query with `>` or `<` on a field that has only a hash index reads the documents as if there were no index; `explain` shows it. A field has one index at a time: to change its kind, drop it and create it again.

From then on the index is kept up to date by every write, automatically. Index definitions are saved and the indexes are rebuilt when KivaDB starts.

```text
kiva> indexes
user:* (role) - 3 documents indexed, 2 distinct values
user:* (name) - hash, 3 documents indexed, 3 distinct values
kiva> drop index on user: (role)
Index on user:* (role) dropped.
```

A write that leaves an indexed field unchanged does not touch that index.

Create an index on fields you filter on often. Each index uses memory and makes writes on matching keys slightly slower, so do not index everything.

### 11.5 `explain` — see how a query will run

```text
kiva> explain get where user:*.role = "dev"
Query Plan
  Filter    : user:*.role = "dev"
  Strategy  : PREFIX SCAN
  Index     : none
  Estimated : 3 of 8 keys examined
  Tip       : create index on user:* (role)
```

After creating the index:

```text
kiva> explain get where user:*.role = "dev"
Query Plan
  Filter    : user:*.role = "dev"
  Strategy  : INDEX SCAN
  Index     : user:* (role)
  Estimated : 2 of 8 keys examined
```

| Strategy | Meaning |
| :--- | :--- |
| `INDEX SCAN` | A secondary index gives the candidates directly |
| `INDEX MERGE` | Several indexes, intersected for `and` and merged for `or` |
| `INDEX ORDER SCAN` | A B-tree index on the field of `order by`, read in order until the `limit` is reached; the condition is checked on the indexes it can be, before a document is read |
| `PREFIX SCAN` | Only the keys sharing the pattern's prefix are examined |
| `FULL SCAN` | Every key is examined (pattern starting with `*`) |
| `ORDERED INDEX RANGE SCAN` | A `range` query: a slice of the sorted keys |
| `POINT LOOKUP` | A `get` on one key |

`explain` also works with the aggregates ([11.3](#113-aggregates-count-sum-avg-min-max-group-by)), `range` and `get <key>`. An index is used for `=`, `>`, `>=`, `<` and `<=`; the `!=` operator always scans.

With several comparisons, KivaDB uses every index it can:

* `and`: the keys of the indexed comparison that gives the fewest are listed (counted first, without being copied). Each of them is then checked against the index of every other comparison, which holds the value of the field for that key, so a document is read only when every index agrees. One indexed comparison is enough; the others are tested on the documents it gives. When the `and` compares with `=` every field of a unique index on several fields ([11.7](#117-unique-indexes)), that index finds the document in one lookup.
* `or`: the keys of each part are merged, but only when **every** part has an index. One part without an index means reading the whole pattern anyway.
* `not`: never an index, since it selects what the index does not list.

Whatever the plan, every candidate document is tested against the whole condition, so an index changes the speed, never the answer. With indexes on `role` and `age`:

```text
kiva> explain get where user:*.role = "dev" and age > 30
Query Plan
  Filter    : user:* where role = "dev" and age > 30
  Strategy  : INDEX MERGE
  Index     : user:* (role) and user:* (age)
  Estimated : 1 of 8 keys examined
kiva> explain get where user:*.role = "dev" or address.city = "Goma"
Query Plan
  Filter    : user:* where role = "dev" or address.city = "Goma"
  Strategy  : PREFIX SCAN
  Index     : none
  Estimated : 3 of 8 keys examined
  Tip       : create index on user:* (address.city)
```

A query with one comparison runs exactly as before: the same index, the same reading.

`explain` also shows the order and the page of a query: `Order` says whether the rows are read in the order of an index or sorted in memory, and `After` and `Limit` give the page. With `order by` and a `limit` but no B-tree on the field, a `Tip` gives the index that would let the query stop early.

#### `explain analyze` — run the query and measure it

`explain` decides the plan from the indexes and reads no document. `explain analyze` **runs the query**: it reads what the query reads and counts it. The rows are not shown; the counts and times are.

```text
kiva> create index on user:* (age)
kiva> explain analyze get where user:*.age > 20 order by age desc limit 2
Query Plan (explain analyze: the query was run; its rows are not shown)
  Filter    : user:*.age > 20
  Strategy  : INDEX ORDER SCAN
  Index     : user:* (age) for the order, user:* (age) checked for the filter
  Order     : age desc, read in the order of the index: stops after 2
  Limit     : 2
  Estimated : 2 of 8 keys examined
Measured
  Index entries : 4
  Keys examined : 2
  Documents     : 2 read (0 from the value cache, 2 from disk), 0 filtered out
  Sorted        : 2 rows in 0.003 ms
  Returned      : 2
  Time          : 0.315 ms (plan 0.014 ms, read and filter 0.287 ms, sort 0.003 ms)
```

Everything under `Query Plan` is decided before the reading (`Estimated` is an estimate). Everything under `Measured` was counted while the query ran:

| Line | Counts |
| :--- | :--- |
| `Index entries` | Keys taken from secondary indexes: listed, walked in order, or checked against an index for one key |
| `Keys examined` | Keys looked at, whether a document was read for them or not (another type, expired, outside the pattern) |
| `Documents` | Documents read, from the value cache ([18](#chapter-18--keeping-values-in-memory-value-cache)) or from the data file, and those that did not pass the condition |
| `Sorted` | Rows put in order in memory, and the time it took; absent without `order by` |
| `Returned` | Rows the query gives; for an aggregate, `Groups` and the documents folded |
| `Time` | The whole query, then choosing the plan, reading and testing the documents, and sorting |

Many `Documents` read against few `Returned` mean that the plan reads documents the condition then rejects: an index on the field tested, or a more selective comparison first, would avoid them. A large `Sorted` with a small `limit` calls for a B-tree index on the field of `order by`.

`explain analyze` works with `get where` and the aggregates (`count`, `sum`, `avg`, `min`, `max`, with `group by`). It costs what the query costs: run it on a copy, or with a `limit`, when the query reads a lot. It needs read access, like the query. It reads what is committed, like `get where`: inside a transaction, not the transaction's own writes. Neither `explain` nor `explain analyze` runs a write:

```text
kiva> explain analyze set x 1
Error: explain analyze never runs a write ('set'): it explains the reads get where, count, sum, avg, min and max
```

A plan shows the condition as written and the names of the indexes, never the values of the documents read.

Over REST, `{"op": "explain", ..., "analyze": true}` answers the same counts under `analyze` ([20.4](#204-structured-requests-for-tools)):

```json
{"plan": {"strategy": "INDEX ORDER SCAN", "index": "user:* (age) for the order, user:* (age) checked for the filter",
          "candidates": 2, "total_keys": 8, "order": "age desc, read in the order of the index: stops after 2"},
 "suggestion": null,
 "analyze": {"index_entries": 4, "keys_examined": 2, "documents_read": 2, "from_cache": 0, "from_disk": 2,
             "filtered_out": 0, "sorted": 2, "returned": 2,
             "time_ms": {"total": 0.274, "plan": 0.023, "read": 0.217, "sort": 0.003}}}
```

**How the plan is chosen.** The plans use what the indexes know, exact and kept up to date by every write: the keys of each value, the number of distinct values and of documents indexed (`indexes` shows them). With a `limit` and a B-tree on the field of `order by`, KivaDB compares reading that index in order with reading the documents the condition selects and sorting them. It counts those documents from the indexes, without copying a key, and stops counting once the answer is clear. The comparison assumes the condition and the order are independent; `explain analyze` shows what was actually read. Choosing never changes the answer, only the reading.

### 11.6 What a query does not do

A query reads **one key pattern**, tests its documents against comparisons with literal values, and returns them in key order or folds them into aggregates. That is a choice: every query is then one pass over indexes or one range of keys, in memory, and `explain` can say in advance what it will cost. KivaDB refuses the rest at once, and says what to do instead:

| Refused | Example | Instead |
| :--- | :--- | :--- |
| Joins | `user:* join order:* on ...`, `user:*, order:*...`, `= order:*.user`, `... and order:*.total > 5` | Keep related data in the same document, or read the related keys by name (`get a and b`) |
| Sorting pipelines | `distinct`, `having`, `offset`, `group by` on `get`, `order by` on an aggregate | `order by <field>` and `after <key>` on `get where` ([11.2](#112-get-where--find-documents)); the aggregates and `group by` ([11.3](#113-aggregates-count-sum-avg-min-max-group-by)); `range` and `keys` return keys in order |
| Other operators | `between`, `contains`, `matches`, `regexp` | The comparisons of [11.1](#111-the-condition): `age >= 20 and age <= 30` for a range, `in [...]` for several values, `like` / `ilike` for text patterns |
| Other combinations | `&&`, `\|\|`, `xor` | `and`, `or`, `not` and parentheses |
| Distributed queries | `@node2`, `across`, `shard`, `explain distributed` | Nothing to do: one server holds all of a workspace's data |

```text
kiva> get where user:*.role = "dev" limit 5 offset 2
Error: 'offset': sorting pipelines (distinct, having, offset, an aggregate sorted...) are not supported: get where sorts with 'order by <field> [desc]' and pages with 'after <key>'; the aggregates are count, sum, avg, min and max, with 'group by <field>'; 'range' and 'keys' return keys in order.
kiva> get where user:*.name = order:*.user
Error: 'order:*.user' names a field of other documents: joins are not supported: a query reads one key pattern and compares with a literal value. Keep related data in the same document, or read the related keys by name (get a and b).
```

Wildcards belong to the key pattern, never to the field (`user:*.na*` is refused). The structured `query` and `aggregate` requests follow the same rules: a request with a field such as `join`, `offset` or `distinct`, or with an array or object as `value` (except a list for `in`), is refused; `and`, `or` and `not` go inside `where` ([20.4](#204-structured-requests-for-tools)).

### 11.7 Unique indexes

A **unique index** refuses a second document holding the same value, whoever writes it: the shell, a program, a Redis client, a script.

```text
create unique index on <pattern> (<field>[, <field>...]) [using btree|hash] [ignore case]
```

```text
kiva> create unique index on user: (name)
Unique index created on user:* (name): 3 documents indexed.
kiva> set user:4 {"name": "Alice", "role": "ops"}
Error: UNIQUE: 'user:4' not written: 'user:1' already holds "Alice" in the unique index on user:* (name)
```

On **several fields**, it is the combination that must be unique: the same name may come back under another parent, not twice under the same one. With **`ignore case`**, texts are compared without regard to capitals, in every alphabet (`Goma`, `GOMA` and `goma` are the same value; so are `Élisabethville` and `élisabethville`):

```text
kiva> create unique index on place: (type, parent, name) ignore case
Unique index created on place:* (type, parent, name) ignore case: 0 documents indexed.
kiva> set place:1 {"type": "VILLE", "parent": null, "name": "Goma"}
OK: place:1 saved.
kiva> set place:2 {"type": "QUARTIER", "parent": "place:1", "name": "Katindo"}
OK: place:2 saved.
kiva> set place:3 {"type": "QUARTIER", "parent": "place:1", "name": "KATINDO"}
Error: UNIQUE: 'place:3' not written: 'place:2' already holds ["quartier","place:1","katindo"] in the unique index on place:* (type, parent, name) ignore case
kiva> set place:4 {"type": "QUARTIER", "parent": "place:9", "name": "Katindo"}
OK: place:4 saved.
kiva> set place:5 {"type": "VILLE", "parent": null, "name": "goma"}
Error: UNIQUE: 'place:5' not written: 'place:1' already holds ["ville",null,"goma"] in the unique index on place:* (type, parent, name) ignore case
```

* **The error** starts with `UNIQUE:` in the shell, the binary protocol and the REST API (code 409), and with `UNIQUE` for Redis clients (`-UNIQUE 'place:3' not written: ...`, [23.14](#2314-unique-indexes-and-add-from-a-redis-client)). Code can test that word instead of the whole message; the Node.js driver gives it as `err.code` ([24.3](#243-nodejs)).
* **What counts.** `null` is a value like the others: two towns with `"parent": null` and the same name conflict. A document that lacks one of the fields (or holds an array or an object there) is not constrained, so leave a field out to let several documents go without it. Without `ignore case`, values compare as in conditions: `"Goma"` and `"goma"` differ. `ignore case` applies to every text field of the index (`type` too, as the errors above show), never to numbers or booleans; the documents keep their own text. So the error shows the value as the index compares it, in small letters (`["quartier","place:1","katindo"]`), not the text stored in either document: `get place:2` still answers `"Katindo"`.
* **When it is checked.** At every write, under the same lock as the write itself: two clients creating the same place at the same instant cannot both succeed, which a "read, then write if absent" in the application cannot promise. Writing a document again with its own value is allowed. A transaction is judged by what its keys hold at `commit`: swapping two values between documents in one transaction works, and a transaction that would leave a duplicate is refused as a whole. A deleted or expired document frees its value.
* **Creating it** is refused while two documents already hold the same value; the error names them. Change one, then create the index again.
* **Queries.** A unique index on one field, without `ignore case`, also answers queries like any index. On several fields, without `ignore case`, it answers an `and` that compares each of its fields with `=` (`place:*.type = "quartier" and parent = "place:1" and name = "katindo"`) in one lookup. With `ignore case`, it only guards the writes: queries use the other indexes, which `and` combines ([11.5](#115-explain--see-how-a-query-will-run)), and `ilike` finds texts without regard to capitals ([11.1](#111-the-condition)).
* `indexes` shows `unique` and `ignore case`; `drop index on place: (type, parent, name)` removes it. Like every index, it is defined per node: create it on the master; a replica copies what the master accepted.
* An index on several fields, or with `ignore case`, that is not unique is refused.

### 11.8 Schemas: rules for the documents of a pattern

Documents are free by default: any field, any type. A schema gives the documents of one key pattern rules that every write must follow, and values filled in when a field is missing. The patterns without a schema work as before.

```text
kiva> schema set user: {"name": {"type": "string", "required": true, "maxLength": 80},
   ...   "email": {"type": "string", "required": true, "unique": true, "pattern": "^[^@ ]+@[^@ ]+$"},
   ...   "age": {"type": "integer", "min": 0, "max": 150},
   ...   "role": {"enum": ["dev", "ops"], "default": "dev"},
   ...   "active": {"type": "boolean", "default": true},
   ...   "created_at": {"type": "datetime", "default": "now()"}}
Schema set on user:* (6 fields).
Unique index created on user:* (email).
kiva> set user:1 {"name": "Ana", "email": "ana@example.org"}
OK: user:1 saved.
kiva> get user:1
user:1: {"name":"Ana","email":"ana@example.org","role":"dev","active":true,"created_at":"2026-10-27T14:30:00.000Z"} [object]
kiva> set user:2 {"name": "Ben"}
Error: SCHEMA: 'user:2' not written: email: required (schema on user:*)
kiva> json set user:1 .age -4
Error: SCHEMA: 'user:1' not written: age: -4 is below the minimum 0 (schema on user:*)
```

The pattern is written as for an index: `user:` means `user:*`. A schema is a JSON object with one rule per field. Fields the schema does not name are free, as before.

| Rule | Meaning |
| :--- | :--- |
| `type` | `string`, `number`, `integer`, `boolean`, `null`, `array`, `object`, `date`, `datetime`, `time` or `any` |
| `required` | The field must be present |
| `nullable` | `null` is accepted besides the type (refused by default when a type is given) |
| `unique` | No two documents of the pattern hold the same value: a unique index ([11.7](#117-unique-indexes)) |
| `min`, `max` | Bounds of a `number` or an `integer` |
| `minLength`, `maxLength` | Length of a `string`, in characters |
| `pattern` | A regular expression a `string` must match (`^` and `$` to match it whole) |
| `enum` | The values allowed, any type |
| `minItems`, `maxItems`, `items` | Size of an `array`, and the rule every item follows |
| `properties` | The rules of the fields of an `object`, nested to 16 levels |
| `default` | The value given to the field when the document lacks it |

In a document, a `date` is the text `"2026-10-27"`, a `time` is `"08:00:00"` or `"08:00"`, and a `datetime` is ISO 8601 **with its offset** (`"2026-10-27T14:30:00Z"`, `"2026-10-27T15:30:00+01:00"`): a moment without its offset would mean a different instant in each time zone. These are the forms that conditions compare as moments ([11.1](#111-the-condition)).

**Defaults.** A default is filled in when the field is missing, never when it holds `null`, and is checked against its rule when the schema is set. Three defaults are computed at the time of the write, for the type they belong to: `"now()"` for a `datetime` (UTC, to the millisecond), `"today()"` for a `date` and `"current_time"` for a `time` (both in the zone of the session writing, [5.9](#59-time-zones)). The same texts are plain values for a field of another type. Nothing else is evaluated: a rule is data, never code.

**What is checked, and when.** Every write of a key of the pattern, whatever it comes from: `set`, `update`, `json set` and the other changes of a field, `add`, the REST API (answered with code `422`), the binary protocol, Redis clients (`HSET`, `JSON.SET`...), Lua scripts and triggers. The whole document is judged after the change, so `json set user:1 .age -4` is refused even though only `age` changed. A refused write changes nothing: the document stays as it was. The error starts with `SCHEMA:` (`SCHEMA` for Redis clients), followed by the key, the field and the reason.

* **Transactions.** The documents are judged at `commit`, like unique indexes: one document that breaks the schema refuses the whole transaction, and nothing of it is written. Every default of one transaction takes the same moment, that of its commit, so documents created together share their `created_at`. Until the commit, the session reads what it wrote as it wrote it: `get` inside the transaction shows the document without its defaults, and an error of the schema comes only at `commit`. Read the document again after the commit to see the values filled in.
* **Documents stored before the schema** stay as they are: setting a schema never changes or deletes them, and they can be read, deleted and have their TTL changed. `schema set` counts those that do not follow it, `schema check user:` lists them with the reason, and each one must follow the schema at its next change (its missing defaults are then filled in).
* **Unique fields** go through a unique index. `schema set` creates it when there is none, and is refused, without changing anything, while two documents share a value or while the field has an index that is not unique. An existing unique index is used as it is. `schema drop` removes the indexes the schema created, and leaves the others.
* **Regular expressions** run in time proportional to the text whatever the expression (no backtracking), are at most 1 000 characters, and their compiled form is bounded: an expression that would need more is refused when the schema is set.
* **Replication.** A replica applies what the master accepted, as it was written: the master judged it and filled in its defaults. Like indexes, schemas are defined per node: set them on each node (`schema set` works on a replica), so that a replica promoted to master keeps them.

| Command | Effect |
| :--- | :--- |
| `schema set <pattern> {...}` | Give the pattern a schema, or replace its schema (admin) |
| `schema show <pattern>` | The schema, as a command to set it again |
| `schemas` | The schemas of the workspace |
| `schema check <pattern>` | The documents that do not follow it |
| `schema drop <pattern>` | Remove it, with the unique indexes it created (admin) |

Schemas are kept with the index definitions, in `store.kiva.meta`. KivaDB 1.x ignores them, and drops them when it saves its index definitions: set them again after going back to 2.0.

---

## Chapter 12 — Vector search for AI

### 12.1 What a vector is, and why store one

AI models turn a text, an image or a sound into an **embedding**: a list of numbers, typically a few hundred long, such as `[0.12, -0.40, 0.88, ...]`. Two embeddings that are close describe things that mean something close. Finding "the documents closest to this one" is how chatbots find the passages that answer a question (RAG), how search engines find results by meaning rather than by exact words, and how recommendations work.

In KivaDB, an embedding is simply stored as an array inside a document:

```text
kiva> set doc:1 {"title": "Kinshasa travel guide", "embedding": [0.9, 0.1, 0.0]}
kiva> set doc:2 {"title": "Goma lake trips", "embedding": [0.8, 0.3, 0.1]}
kiva> set doc:3 {"title": "Rust programming", "embedding": [0.0, 0.2, 0.95]}
```

Real embeddings come from a model (OpenAI, Mistral, a local model...) and have 384, 768, 1536 or more numbers; three are used here to keep the examples readable. Every vector compared in one search must have the same length.

### 12.2 Searching

```text
get similar <pattern>.<field> to <vector> [limit N] [metric cosine|dot|l2] [ef N] [exact] [return f1, f2]
```

```text
kiva> get similar doc:*.embedding to [1, 0, 0] limit 2 return title
doc:1: 0.9939 {"title":"Kinshasa travel guide"}
doc:2: 0.9300 {"title":"Goma lake trips"}
(2 results, cosine metric, 3 vectors compared, documents read from disk)
```

Each line gives the key, the score, and the document (or the fields named after `return`). Results come best first; `limit` defaults to 10.

**"More like this".** Instead of a list of numbers, give the vector of another document:

```text
kiva> get similar doc:*.embedding to doc:3.embedding limit 2 metric l2 return title
doc:3: 0.0000 {"title":"Rust programming"}
doc:2: 1.1715 {"title":"Goma lake trips"}
(2 results, l2 metric, 3 vectors compared, documents read from disk)
```

### 12.3 Metrics

| Metric | Measures | Best score | Use when |
| :--- | :--- | :--- | :--- |
| `cosine` (default) | The angle between the vectors | 1 (same direction); -1 is opposite | Almost always: text embeddings are meant for it |
| `dot` | The dot product | Higher is closer | The model's documentation asks for it (vectors already normalised) |
| `l2` | The straight-line distance | 0 (identical); lower is closer | Image or numeric features where distance matters |

### 12.4 Vector indexes

Without an index, a search reads every matching document from disk. A **vector index** keeps the vectors of a field in memory, as 32-bit floats, and keeps them up to date on every write:

```text
kiva> create vector index on doc: (embedding)
Vector index created on doc:* (embedding): 3 vectors in memory.
kiva> get similar doc:*.embedding to [0, 0, 1] limit 1
doc:3: 0.9785 {"title":"Rust programming","embedding":[0.0,0.2,0.95]}
(1 results, cosine metric, 3 vectors compared, vector index)
kiva> indexes
doc:* (embedding) - vector index, 3 vectors, 3 dimensions, 243 bytes in memory
Vector memory: 243 bytes of 8.00 GB (0.0%, vector.max_ram_mb)
```

* The first vector indexed fixes the number of dimensions. Documents whose vector has another length are skipped and counted (`indexes` shows how many). So are documents whose field holds something else than numbers, such as the text itself: KivaDB does not compute embeddings ([12.11](#1211-what-the-vector-search-leaves-to-your-application)).
* Memory needed: about 4 bytes per number, so 100 000 vectors of 768 dimensions take about 300 MB. `indexes` shows what each index takes, and the bound of [12.10](#1210-memory-bounds) keeps the total within the machine's RAM.
* The definition is saved; the index is rebuilt when KivaDB starts. `drop vector index on doc: (embedding)` removes it.

This index is **exact**: every vector is compared, so the results are always the true closest ones.

### 12.5 Approximate index (HNSW) for large collections

Comparing every vector costs time in proportion to their number. Beyond about a hundred thousand vectors, an **HNSW index** (*Hierarchical Navigable Small World*) answers much faster by comparing only a small part of them:

```text
kiva> create vector index on doc: (embedding) using hnsw
HNSW vector index created on doc:* (embedding): 250000 vectors in a graph (cosine, m 16, ef 200).
kiva> get similar doc:*.embedding to [0.12, -0.4, ...] limit 5 return title
...
(5 results, cosine metric, 676 vectors compared, HNSW index, approximate)
```

**How it works.** Each vector becomes a point of a graph, linked to a few of its closest neighbours. The graph has layers, like the express lines of a metro: the top layer holds a few points far apart, each lower layer more of them, the bottom one all. A search starts at the top, jumps towards the query on each layer, then explores the neighbourhood it reached on the bottom layer, keeping the `ef` best candidates. It looks at a few thousand vectors instead of all of them.

**The price: approximate results.** Now and then, one of the true closest vectors is missed and the next one is returned instead. How often depends on the data. Embeddings gather around topics, and on such vectors the defaults found 100 % of the true 10 closest in our measurements (12.6). On vectors spread evenly at random, the hardest case, they found 28 %; `ef 800` brought it to 87 %, at the cost of slower searches. Check on your own data: compare a few searches with and without `exact`, and raise `ef` if too much is missed.

| Option | Where | Default | Effect |
| :--- | :--- | :--- | :--- |
| `metric cosine|dot|l2` | `create` | `cosine` | The metric the graph is built for. Searches with another metric compare every vector |
| `m N` | `create` | 16 (2 to 100) | Links per point. More: better results and more memory |
| `ef N` | `create` | 200 (10 to 2000) | Candidates while building. More: a better graph, built more slowly |
| `ef_search N` | `create`, `tune` | 100 | Candidates a search keeps when it does not say: the index's own default. `tune` measures it ([12.12](#1212-tuning-an-hnsw-index)) |
| `quantize sq8\|none` | `create` | `none` | `sq8` keeps one byte per number instead of four ([12.13](#1213-quantization-a-quarter-of-the-memory-for-the-vectors)) |
| `ef N` | `get similar` | the index's `ef_search` | Candidates while searching, for this search. More: fewer misses, slower |
| `exact` | `get similar` | | Compare every vector this time, for example to check the results |

```text
kiva> create vector index on img: (features) using hnsw metric l2 m 32 ef 400
kiva> get similar img:*.features to [...] limit 10 ef 300
kiva> get similar img:*.features to [...] limit 10 exact
```

* **Writes.** A new or changed vector is added to the graph at once; a deleted or replaced one stays in it as a mark, still used to find the way but never returned. When those marks reach a third of the graph, the maintenance task rebuilds it from the live vectors; on a large graph this takes a while, and the commands of that workspace wait meanwhile. Changing another field of a document does not touch the graph.
* **Restarts.** The graph is saved next to the data (`<file>.graphs`) when the index is created, when KivaDB stops, after a compaction, and from time to time after many changes. At start, KivaDB reads it back and replays only the writes made after it was saved, instead of building the graph again. A missing or damaged file is not an error: the graph is built again from the documents. The format is described in [12.12](#1212-tuning-an-hnsw-index).
* **When it is used.** A search uses the graph when its pattern is the index's own (`doc:*` for an index on `doc:`) and its metric is the graph's. A narrower pattern (`doc:2026*`) or another metric compares every vector of the index, exactly, as in 12.4.
* **Building.** Creating the index builds the graph at once, on one processor core, and the workspace waits meanwhile: about 0.6 ms per vector of 128 dimensions with the defaults, more with longer vectors or a higher `ef`. Create it before loading millions of documents, or at a quiet time. Afterwards, each write adds its vector to the graph in about the same time.
* **Memory.** The vectors (4 bytes per number) plus the links (about `m` × 8 bytes per vector): 1 million vectors of 768 dimensions take about 3.2 GB, about 1.1 GB with `quantize sq8` ([12.13](#1213-quantization-a-quarter-of-the-memory-for-the-vectors)).
* `indexes` shows the kind of each index and its settings.

### 12.6 How fast

Measured on an ordinary laptop (release build, one search at a time, 10 results), 100 000 vectors of 128 dimensions, HNSW with the defaults:

| Vectors | Index | Vectors compared | Time per search | True 10 closest found |
| :--- | :--- | :--- | :--- | :--- |
| Grouped around topics, like embeddings | Exact | 100 000 | 105 ms | all |
| | HNSW | 676 | 0.5 ms | 100 % |
| Spread evenly at random (hardest case) | Exact | 100 000 | 105 ms | all |
| | HNSW | 2 983 | 1.1 ms | 28 % (87 % with `ef 800`) |

Building the graph took 57 s (155 s for the evenly spread vectors); stopping and restarting KivaDB with the saved graph took 0.5 s, instead of building it again. The exact search grows with the number of vectors; the HNSW search barely does. Rule of thumb: up to about 100 000 vectors, the exact index is fast enough and never misses; beyond, use HNSW. The test `hnsw_speed` in `tests/power.rs` repeats the measurement on your machine (`cargo test --release --test power hnsw_speed -- --ignored --nocapture`; `HNSW_N` and `HNSW_DIMS` set the sizes, `HNSW_DATA=uniform` the evenly spread vectors).

### 12.7 From programs

The structured request `similar` returns the same results as JSON (`{"op": "similar", "target": "doc:*.embedding", "vector": [...], "limit": 5, "metric": "cosine"}`, plus `"ef"` and `"exact"`; the answer says `"approximate": true` when an HNSW index answered), on port 2911 or with `POST /v1/api`. A `"filter"` keeps only some documents ([12.9](#129-filtering-on-other-fields)): `{"lang": "fr"}` for equalities, or `[{"field": "year", "cmp": ">=", "value": 2020}]` for comparisons; the answer's `"filter"` says how it was applied. The Node.js driver has `db.similar(target, vector, {limit, metric, ef, exact})`, and Kiva Studio has a Similarity tab in its Query screen.


### 12.8 Semantic cache: reuse answers to the same question

Calling a language model costs time (often seconds) and money, and users ask the same things again and again, rarely with the same words: *"What is the capital of the DRC?"*, *"capital of Congo-Kinshasa?"*. The **semantic cache** keeps each answer next to the embedding of its question; a later question whose embedding is close enough gets that answer back, in a fraction of a millisecond, without calling the model.

KivaDB does not compute embeddings: your application already does (it needs them for its own searches), and gives the vector to KivaDB. A question repeated word for word is also found by its **text** alone (ignoring case and extra spaces), before any embedding is computed.

```text
kiva> semantic lookup faq vector [0.12, -0.40, ...]
Miss (closest similarity 0.6000, threshold 0.98; since 1792966517756800)
                       ...the application calls its model, then:
kiva> semantic store faq text "What is the capital of the DRC?" vector [0.12, -0.40, ...] answer "Kinshasa" depends "geo:*" since 1792966517756800
Stored as faq:1 (expires in 300 s).
kiva> semantic lookup faq vector [0.12, -0.39, ...]
Hit (similarity 0.9950, faq:1): "Kinshasa"
kiva> semantic lookup faq text "what is the capital of the DRC?"
Hit (similarity 1.0000, faq:1): "Kinshasa"
```

| Command | Effect |
| :--- | :--- |
| `semantic lookup <cache> [text "<q>"] [vector [...]] [threshold <0-1>]` | The stored answer of this question or a near-identical one (`Hit`), or `Miss` with the closest similarity seen |
| `semantic store <cache> [text "<q>"] [vector [...]] answer <value> [ttl <sec>] [depends "<pattern>", ...] [since <token>]` | Remember an answer (any value: text, number, JSON document); with `since`, only if the data it depends on did not change after the lookup |
| `semantic stats` | Entries, hits, misses and hit rate of each cache, answers dropped by writes, distinct dependencies |
| `semantic clear [<cache>]` | Forget one cache, or every cache of the workspace |

* **Several caches**, by name (`faq`, `support`, `llm`...), per workspace. A cache holds vectors of one length: those of one model.
* **Similarity**: cosine, through an HNSW graph ([12.5](#125-approximate-index-hnsw-for-large-collections)). `semantic.cache.threshold` (0.98) decides what counts as the same question; lower it and different questions start to share answers. The right value depends on your embedding model: look at the similarities `semantic lookup` reports for a few rewordings and a few different questions, and set the threshold between the two.
* **Freshness**: an answer lives `semantic.cache.ttl` seconds (300; `ttl` per entry, `ttl 0` = until invalidated). Storing the same question again replaces its answer. Beyond `semantic.cache.max_entries` (10 000 per cache), the oldest answers go first.
* **Dependencies**: an answer built from your data names it with `depends`, as keys (`"kb:42"`) or patterns (`"kb:*"`, `"user:?"`). As soon as a matching key is written, deleted, or expires with its TTL (at the latest a second after its deadline), the answer is dropped, by any client: shell, structured requests, Redis commands, transactions, triggers. `del all keys` (`FLUSHDB`) drops every answer of the workspace. A write costs the cache one hash lookup for all the plain keys together, and one per distinct length of the text before the wildcard of the patterns (`chunk:` in `chunk:*`), whatever the number of answers and patterns: only the patterns whose text begins the key are compared with it. Measured in a release build (test `invalidation_speed`): 0.4 µs per write with 10 000 answers depending on 10 100 keys and patterns, 0.6 µs when each of the 10 000 answers has a pattern of its own (`chunk:<n>:*`), against 435 µs when every pattern was compared and 14.7 ms when every answer was.
* **Data that changes while the model thinks**: between the lookup that missed and the store, the model works, often for seconds. If a document the answer depends on is written meanwhile, the answer was built from its old version, and storing it would keep a wrong answer until its TTL. Give the store the `since` token the miss printed: if a key matching `depends` changed after it, the answer is not kept (`Not stored: kb:42 changed after the lookup...`), and the next question goes to the model with the new data. Without `since`, the answer is kept as before. A token older than the last 10 000 writes of the workspace, or from before a restart or a `del all keys`, can no longer be checked: the store is refused rather than guessed. `get similar` with `semantic.cache.enabled` does this by itself.
* **Caching `get similar` itself**: with `config set semantic.cache.enabled true`, a vector search near-identical to one made recently (same pattern, field, metric and limit) (and the same filter) returns the earlier results, marked `semantic cache: a query at similarity ... was answered earlier`; any write under the pattern makes them stale, also one made during the search, whose results are then not kept. `exact` always computes. The results are those of the earlier query, close but not necessarily identical: leave it off when every search must be computed.
* The cache lives in memory: it is not saved, replicated or backed up, and fills again after a restart.

**How much it saves.** Measured (release build) on 2 000 cached questions of 128 dimensions: a lookup takes about 0.13 ms. Of 500 questions (400 rewordings of known ones, 100 new), the 400 rewordings were all recognised with the right answer and the 100 new ones all went to the model: 80 % fewer model calls. The test `a_cache_in_front_of_a_slow_model_saves_most_of_its_calls` in `tests/semantic_cache.rs` repeats it.

### 12.9 Filtering on other fields

A search often concerns only part of the documents: those in one language, of one customer, from recent years. `where` keeps the documents whose other fields pass every condition, joined by `and`:

```text
kiva> get similar doc:*.embedding to [1, 0, 0] where lang = "fr" and year >= 2020 limit 5 return title
doc:1: 0.9938 {"title":"Kinshasa guide"}
(1 results, cosine metric, 3 vectors compared, vector index; filter tested on the closest vectors first, 2 documents read)
```

* A condition is `<field> <op> <value>` with the operators of [11.1](#111-the-condition) (`= != > >= < <=`, `in`, `like`, `ilike`; not `not in`). The field belongs to the documents searched; it may repeat their pattern (`doc:*.lang`). A document without the field does not pass.
* Only `and`: the filter is simple on purpose. For `or`, run one search per alternative and merge the results. Text search and sparse (keyword) vectors combined with the dense ones are not part of it.
* The last line says how the filter was applied, chosen by KivaDB:

| Situation | How | Result |
| :--- | :--- | :--- |
| A secondary index ([11.4](#114-indexes--making-queries-fast)) serves one of the conditions, and leaves few documents | **Pre-filtering**: only the documents that index gives are compared, after the other conditions | Exact, even with HNSW |
| An exact vector index | **Post-filtering**: the vectors are ranked, then the documents are read closest first until enough pass | Exact |
| An HNSW index | **Filtered walk**: the condition is tested while the graph is explored. Nodes that fail it are still walked through, but only those that pass fill the list of `ef` best candidates, however rare they are. The conditions a secondary index serves are tested against its keys, without reading anything; the others read the document, once per node close enough to count | Approximate |
| No vector index | Each document read is tested before its vector is compared | Exact |

With HNSW, "few" means at most a tenth of the graph: a narrower filter is answered faster and exactly by comparing its documents, a broader one by the graph walk.

**Give the filtered fields a secondary index.** In a filtered walk, a condition without an index reads a document for each node it tests; with one, it reads none. Measured on 100 000 vectors of 128 dimensions grouped around 200 topics, each topic entirely in one group (the hard case: the closest vectors all fail the filter), 50 queries, release build:

| Filter | Before (search, then filter, then search again for more) | Filtered walk |
| :--- | :--- | :--- |
| One document in five, no index | 95.2 % of the true 10 closest, 41 ms, 1 397 documents read | 96.4 %, 54 ms, 2 087 documents read |
| One in a hundred, no index | 96.6 %, 640 ms | 99.8 %, 707 ms |
| One in five, **index on the field** | 95.2 %, 48 ms, 1 397 documents read | 96.4 %, **8.8 ms, 10 documents read** |

Without an index, the walk finds more of the true closest vectors but reads more documents. With an index, it reads only the documents it returns. The test `hnsw_filter_speed` in `tests/power.rs` repeats the measurement.

From programs: `"filter"` in the structured request ([12.7](#127-from-programs)), `FILTER <field> <op> <value>` (repeatable) in `KIVA.SIMILAR` ([23.10](#2310-vector-search-and-the-semantic-cache-from-a-redis-client)).

### 12.10 Memory bounds

Vector indexes live in memory only. So that they cannot exhaust the machine's RAM, `vector.max_ram_mb` bounds the memory the vector indexes of a workspace may take:

| Value | Bound |
| :--- | :--- |
| `auto` (default) | Half the machine's RAM, or half the memory limit of the container (Docker, Kubernetes) when it is lower |
| A number | That many megabytes |
| `0` | None of their own (`storage.max_memory_mb` still counts them) |

* **Writes.** A write that would add a vector past the bound is refused, and nothing is written:
  ```text
  Error: OOM: the vector indexes of this workspace reached their memory bound (16.00 KB of 16.00 KB used), so new vectors are refused. Delete documents, drop a vector index, or raise vector.max_ram_mb.
  ```
  What adds no vector always goes through: replacing the vector of a document in an exact index, documents without a vector, other keys, deletes. A deleted vector gives its memory back at once in an exact index, at the next rebuild of the graph with HNSW.
* **Creating an index.** The build stops as soon as it passes the room left, and the index is not created: `the vector index on big:* (embedding) stopped building after <n> vectors in <size>. OOM: ...`. An index too large is refused before it fills the memory.
* **`storage.max_memory_mb`** ([18.6](#186-a-memory-bound)) counts the vectors too, and refuses a vector past it even for a document that already exists.
* **What is never refused**: loading the indexes at start, and what a master sends to its replica.
* `indexes` and `memory` show the memory used and the bound. `config set vector.max_ram_mb 4096` applies at once.
* The figures are estimates that count each graph link as full, so the real use stays below them.

### 12.11 What the vector search leaves to your application

KivaDB's vector search is that of a database that also holds the documents, not of a dedicated vector engine. Its limits are deliberate:

* **Embeddings come from your application.** KivaDB stores and compares the vectors it is given, as arrays of numbers; it never calls a model. Text sent where a vector is expected is refused with `KivaDB does not compute embeddings: ...`, in the shell, the structured requests and `KIVA.SIMILAR`. Compute vectors with your model (OpenAI, Mistral, Ollama, sentence-transformers...) and store them in a field of the document. One index holds vectors of one length: those of one model.
* **In memory, on one machine.** The vectors of an index, exact or HNSW, are in RAM: the graph is saved to disk to start faster, but searched in memory. `quantize sq8` keeps one byte per number instead of four ([12.13](#1213-quantization-a-quarter-of-the-memory-for-the-vectors)); there is no product quantization, and no graph searched from disk (DiskANN). A workspace is not split across machines (no sharding): replicas hold full copies, for reading and failover ([Chapter 25](#chapter-25--replication)). The collection must fit in the RAM of one machine: about 4 bytes per number, plus the graph links ([12.5](#125-approximate-index-hnsw-for-large-collections)), within `vector.max_ram_mb`.
* **Simple filters.** Conditions joined by `and` ([12.9](#129-filtering-on-other-fields)); no `or`, no full-text search, no hybrid search mixing keyword and dense vectors.

As a rule of thumb, up to a few million vectors of a few hundred dimensions, on one machine, with filters on a few fields, KivaDB covers the need without a second system. Beyond, or for hybrid search, a dedicated vector engine is the right tool, and KivaDB can keep the documents.

**From programs.** The structured request `{"op": "semantic", "action": "lookup" | "store", "cache": "faq", "text": "...", "vector": [...], "threshold": 0.98, "answer": ..., "ttl": 300, "depends": ["geo:*"], "since": 1792966517756800}`. A lookup answers `{"hit": true, "answer": ..., "similarity": 0.995, "id": "faq:1"}` or `{"hit": false, "best": 0.6, "since": 1792966517756800}`; a store answers `{"id": "faq:1", "stored": true}`, or `{"id": null, "stored": false, "stale": "kb:42 changed"}` when its data changed after `since`. In Node.js, `semanticCached` does the three steps:

```js
const answer = await db.semanticCached("faq", { text: question, vector: embedding, depends: ["kb:*"] },
  () => askTheModel(question));
```
---

### 12.12 Tuning an HNSW index

How many of the true closest vectors HNSW finds depends on the data, and the right `ef_search` (candidates kept per search) cannot be guessed: on vectors grouped by topic, 25 is plenty; on evenly spread ones, even 800 misses one in eight. `tune` measures it on your data and keeps the smallest value that finds the share you ask for:

```text
kiva> tune vector index on chunk: (embedding) recall 0.95
Tuning chunk:* (embedding), 100000 vectors: 100 queries taken from its vectors, each searched without its own node, 10 closest each
  exact search       : 89510 µs per search, every vector compared
  ef_search 10       : recall  72.3 %, 320 vectors compared, 36 µs per search
  ef_search 16       : recall  84.1 %, 387 vectors compared, 49 µs per search
  ef_search 25       : recall  93.5 %, 458 vectors compared, 67 µs per search
  ef_search 40       : recall  98.0 %, 534 vectors compared, 72 µs per search
Kept ef_search 40 for chunk:* (embedding): the smallest that finds 95 % of the true closest vectors.
```

* **How it measures.** It takes `sample` vectors spread over the index (100), answers each one exactly by comparing every vector, then with the graph for growing values of `ef_search` (10, 16, 25, 40, 64, 100, 160... up to 4000), and stops at the first that finds `recall` (0.95) of the `limit` (10) true closest. An indexed vector would be too easy a query, since the search would land on its own node, whose links lead straight to its neighbours; each one is searched as if its node were not in the graph (*leave-one-out*), as a new query resembling the data.
* **Your own queries.** When your application keeps real queries (questions asked, with their embeddings), `queries "<pattern>"` uses those documents instead: `tune vector index on chunk: (embedding) queries "query:*"`. They must hold a vector in the same field.
* **How close the estimate is.** Measured on 100 000 vectors of 128 dimensions (release build), compared with 100 real queries:

| Vectors | `tune` alone keeps | With the real queries | Recall of the real queries at the kept value |
| :--- | :--- | :--- | :--- |
| Grouped around topics | 40 | 25 | over 95.2 % (already reached at 25) |
| Spread evenly at random | 1600 | 1600 | 95.4 % |

  The estimate errs on the side of caution: it may keep a larger value than needed, which costs some speed, rarely a smaller one.
* **What it keeps.** The value is saved with the index, survives restarts, and is used by every `get similar`, `KIVA.SIMILAR` and structured `similar` request that does not give its own `ef`. `indexes` shows it: `HNSW vector index (cosine, m 16, ef 200, ef_search 40)`.
* **When no value reaches the recall**, it keeps the largest it tried and says so: build the graph with more links (`using hnsw m 32 ef 400`), or use `exact`.
* **By hand.** `tune vector index on chunk: (embedding) ef_search 200` sets it, `ef_search default` goes back to 100, and `create vector index ... using hnsw ef_search 200` sets it at creation. `ef_search` only changes searches: the graph is not rebuilt.
* **Cost.** Each query is also answered exactly, so tuning takes about `sample` exact searches (9 s on 100 000 vectors). The workspace keeps working meanwhile: tuning takes the lock for one search at a time and lets the waiting writes and reads go first, so a write waits at most about one exact search (7.6 ms at worst during a tuning of 4 000 vectors of 64 dimensions, test `tuning_lets_the_workspace_write_meanwhile`). It needs the admin role. An exact index has nothing to tune: it always finds the true closest vectors.

**Kept tuned as the data grows.** The `ef_search` a graph needs grows with the number of vectors: a value measured on 10 000 vectors misses more on a million. With `auto`, the engine measures it again by itself:

```text
kiva> tune vector index on chunk: (embedding) recall 0.95 auto
...
Kept ef_search 40 for chunk:* (embedding): the smallest that finds 95 % of the true closest vectors.
It is measured again in the background when the index reaches 200000 or falls to 50000 vectors.
kiva> indexes
chunk:* (embedding) - HNSW vector index (cosine, m 16, ef 200, ef_search 40), 100000 vectors, ..., ef_search tuned again as it grows (recall 95 %, last at 100000 vectors)
```

* The recall, `limit`, `sample` and `queries` are saved with the index. Each time the number of vectors doubles or halves since the last tuning, a write starts a new tuning in the background, one at a time per workspace, which keeps its new `ef_search`.
* On an index still empty or nearly, `tune ... auto` answers that it will tune once there are more than `limit` vectors, and does it then.
* `tune ... auto off` keeps the current value from then on; so does `tune ... ef_search N`, a value chosen by hand.
* A new tuning measures the recall, it does not guess it: the index must hold vectors resembling the queries (or the documents named by `queries`). Between two tunings, a value measured on half the vectors may already miss a little more than asked.

**The graphs file.** Graphs are saved to `<file>.graphs` in a binary format of their own, read back at start instead of being built again (0.6 s for 100 000 vectors of 128 dimensions, against 57 s to build):

| Part | Content |
| :--- | :--- |
| Header | `KVGRAPH1`, the identity of the data file, and the point of the log the graphs reflect |
| Per graph | Pattern, field, dimensions, skipped documents, metric, `m`, `ef`, entry point, and every node: key, vector, deleted mark, links per layer |
| End | A CRC32 checksum of everything before it |

It is written to `<file>.graphs.tmp`, synced to disk, then renamed, so a crash leaves either the old file or the new one, never half of one. At start, a file that belongs to another data file, reflects a point the log no longer reaches, fails its checksum, or holds a link to a node that does not exist, is ignored and the graph is built again; one that is valid is brought up to date by replaying only the writes made after it. With encryption at rest, the file is encrypted with the data ([16.12](#1612-encryption-at-rest)). The index settings themselves (`m`, `ef`, `ef_search`) live in `<file>.meta`.

### 12.13 Quantization: a quarter of the memory for the vectors

An HNSW index keeps every vector in memory, 4 bytes per number. With `quantize sq8` (*scalar quantization, 8 bits*), it keeps one byte per number:

```text
kiva> create vector index on doc: (embedding) using hnsw quantize sq8
HNSW vector index created on doc:* (embedding): 250000 vectors in a graph (cosine, m 16, ef 200, sq8: one byte per number, re-ranked with the documents' vectors).
```

* **How.** Each vector is stored as its smallest and largest number and, for each number, its place between the two on a scale of 256 steps. The graph computes its distances on those bytes directly. They are close to the true distances, not equal.
* **The true order.** A search asks the graph for twice the results wanted, reads their documents, and orders them with the full vectors, which stay on disk in the documents. The scores returned are therefore the exact ones, as with `exact`, and a search reads twice as many documents.
* **`exact`** compares the full vectors of every document, read from disk, not the bytes kept in memory. It is slower than an exact search on an index without quantization.
* **Saved** with the index (`.meta`), in the graphs file, and across restarts.

Measured on 100 000 vectors of 128 dimensions, 50 queries, release build:

| Vectors | Index | Memory | True 10 closest found | Per search |
| :--- | :--- | :--- | :--- | :--- |
| Grouped around topics | full (4 bytes) | 81 MB | 100 % | 0.79 ms |
| Grouped around topics | `sq8` | 44 MB | 100 % | 0.85 ms |
| Spread evenly (hardest case) | full | 81 MB | 29.2 % | 1.48 ms |
| Spread evenly | `sq8` | 44 MB | 30.4 % | 2.25 ms |

At 128 dimensions, the links of the graph weigh as much as the vectors. With 768 dimensions the vectors dominate: 1 million of them take about 3.2 GB in full, about 1.1 GB with `sq8`. On evenly spread vectors, quantization loses nothing: the misses come from the graph, and `ef` or `tune` fix them ([12.12](#1212-tuning-an-hnsw-index)). The test `hnsw_sq8_speed` in `tests/power.rs` repeats the measurement.

**Not offered:** product quantization (PQ), which shrinks vectors 10 to 30 times further but needs codebooks trained on the data, retrained as it changes, and loses more accuracy. Nor graphs searched from disk (DiskANN), which need another kind of graph and fast SSDs. With `sq8`, the bytes in memory serve the graph walk, and the full vectors on disk serve the final order.

## Chapter 13 — Transactions

### 13.1 The problem

Imagine moving money: take 50 from one account, add 50 to another. If the program stops between the two steps, 50 has vanished. A **transaction** groups changes so that they all happen, or none of them does.

### 13.2 begin, commit, rollback

```text
kiva> set account:a 100 and account:b 100
OK: account:a saved.
OK: account:b saved.
kiva> begin
Transaction 2 started.
kiva> update account:a 50
OK: account:a updated.
kiva> update account:b 150
OK: account:b updated.
kiva> commit
Transaction committed (2 keys written).
```

Between `begin` and `commit`, nothing is written to the database yet: the changes are kept aside. Other users do not see them. `commit` applies them all in one step; `rollback` throws them away:

```text
kiva> begin
Transaction 3 started.
kiva> update account:a 0
OK: account:a updated.
kiva> rollback
Transaction rolled back.
kiva> get account:a
account:a: 50 [number]
```

Inside your transaction, `get`, `has`, `typeof`, `json` and `print` see your pending changes. Listing and query commands (`keys`, `scan`, `range`, `where`) show what is committed.

### 13.3 Savepoints

A **savepoint** is a bookmark inside a transaction. You can come back to it without abandoning what was done before it.

```text
kiva> begin
Transaction 4 started.
kiva> set order:1 {"total": 50}
OK: order:1 saved.
kiva> savepoint before_delete
Savepoint 'before_delete' created.
kiva> del colors
Deleted: colors
kiva> rollback to before_delete
Rolled back to savepoint 'before_delete'.
kiva> commit
Transaction committed (1 keys written).
```

`order:1` was written; the deletion of `colors` was cancelled.

### 13.4 Status

```text
kiva> transaction status
Transaction ID: 4
State: ACTIVE
Savepoints: 1
Modified keys: 2
```

Outside a transaction it answers `State: NONE (no active transaction)`.

### 13.5 Errors inside a transaction

If a data command fails inside a transaction, the whole transaction is cancelled. You never end up with half of your changes applied by accident:

```text
kiva> begin
Transaction 5 started.
kiva> update account:a 1
OK: account:a updated.
kiva> update account:a "text"
Error: Cannot update [number] with a value formatted as [string].
Transaction rolled back (an error occurred inside the transaction).
```

### 13.6 Conflicts between users

Two people may open a transaction at the same time and touch the same key. KivaDB lets both work, and checks at `commit`: if a key of your transaction was modified by someone else since your `begin`, your commit is refused and nothing of your transaction is applied.

```text
Terminal A                          Terminal B
----------                          ----------
begin                               begin
update balance 10                   update balance 20
commit        -> committed
                                    commit
                                    Error: ERR_TRANSACTION_CONFLICT: key 'balance'
                                    was modified by another session
```

When that happens, start again: `begin`, read the fresh values, redo the changes, `commit`. This approach is called **optimistic** concurrency: no one waits for anyone, and conflicts, which are rare, are detected at the end.

### 13.7 Guarantees

* **Atomic on disk.** A committed transaction is written as one batch whose last record marks the commit. If the machine stops in the middle, the incomplete batch is ignored at the next start.
* **Leaving cancels.** Closing the shell or losing the connection with a transaction open is a rollback.
* `del all keys` and `workspace switch` are refused inside a transaction.

---

## Chapter 14 — Workspaces

A **workspace** is an independent database: its own keys, its own files, its own indexes. Use one per project, or one for tests and one for real data.

### 14.1 The default workspace

There is always a workspace named `default`. It is where an administrator lands.

### 14.2 Creating and switching

```text
kiva> workspace create shop
Workspace 'shop' created at C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop.
kiva> workspace switch shop
Switched to workspace 'shop' (0 keys).
kiva> set product:1 {"name": "Pen", "price": 2}
OK: product:1 saved.
kiva> workspace list
   default              write  C:\Users\you\AppData\Roaming\KivaDB\data
 * shop                 write  C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop
kiva> workspace switch default
Switched to workspace 'default' (11 keys).
```

The `*` marks the workspace you are in. `workspace current` shows only that one.

To keep a workspace somewhere specific (another disk, a project folder), give the path:

```text
kiva> workspace create projet_a "D:\projects\a\db"
```

### 14.3 Opening a workspace directly

```bash
kivadb -w shop
```

### 14.4 Removing

```text
kiva> workspace remove shop
Workspace 'shop' unregistered. Its data files were left on disk.
```

KivaDB forgets the workspace but does not delete its files; remove the folder yourself if you want the data gone. The `default` workspace and the one you are currently in cannot be removed.

### 14.5 Things to know

* Workspace names may contain letters, digits, `_` and `-`, and one `.` after the name of a project (`travel.places`, [14.6](#146-projects)).
* Creating and removing workspaces requires the admin role. In a project, the users who can write in all of it may create its workspaces too ([16.4](#164-workspaces-and-grants)).
* A user reaches only the workspaces they were granted ([16.4](#164-workspaces-and-grants)); a new account reaches none.
* Stored scripts, triggers and users are shared by all workspaces. Indexes belong to one workspace.

### 14.6 Projects

When one server holds several applications, their workspaces pile up: `places_travel`, `places_school`, `cache_travel`... A **project** groups the workspaces of one application under its name. A workspace in a project is written `<project>.<workspace>`, and two projects may each have a workspace of the same name: `travel.places` and `school.places` are two workspaces, with nothing in common.

```text
kiva> project create travel
Project 'travel' created. Its workspaces are named travel.<name>: workspace create travel.places
kiva> workspace create travel.places
Workspace 'travel.places' created at C:\Users\you\AppData\Roaming\KivaDB\workspaces\travel.places.
kiva> workspace create travel.cache
Workspace 'travel.cache' created at C:\Users\you\AppData\Roaming\KivaDB\workspaces\travel.cache.
kiva> project create school
kiva> workspace create school.places
kiva> project list
 school               1 workspace(s): places
 travel               2 workspace(s): cache, places
kiva> workspace list travel
   travel.cache         write  C:\Users\you\AppData\Roaming\KivaDB\workspaces\travel.cache
   travel.places        write  C:\Users\you\AppData\Roaming\KivaDB\workspaces\travel.places
```

* A workspace is used like any other: `workspace switch travel.places`, `kivadb -w travel.places`, `?workspace=travel.places` over HTTP, `SELECT travel.places` or the login `app@travel.places` for Redis clients ([23.4](#234-logins-databases-roles)).
* Projects are optional: `default` and the workspaces created without a project stay as they are.
* A grant can name a whole project: `grant write on travel.* to "app"` covers `travel.places`, `travel.cache` and the workspaces created in `travel` later ([16.4](#164-workspaces-and-grants)).
* `project create` and `project remove <name>` need the admin role. A project is removed only once it holds no workspace: move or remove them first. `project list` shows a user the projects where they can reach something.
* Project names may contain letters, digits, `_` and `-`. Projects are kept in the system database, so replicas know them too.

### 14.7 Renaming or moving a workspace

`workspace move` gives a workspace another name, which also moves it into a project, out of one, or from one to another:

```text
kiva> workspace move places_travel to travel.places
Workspace 'places_travel' is now 'travel.places'. Its files did not move, and its grants followed it.
```

* The data stays in the same folder; only the name changes. The grants on that workspace, and the users whose default workspace it was, follow it. A grant on a whole project (`travel.*`) applies to what the project holds, so moving a workspace out of `travel` takes it out of that grant.
* It needs the admin role, and is refused while a session is in the workspace (`workspace 'places_travel' is in use`), as for the one you are in: run it when the applications are stopped, then give them the new name.
* Applications that named the old workspace (`?workspace=`, `SELECT`, `kivadb -w`) must use the new name.
* Replicas are disconnected and copy the master again, under the new names.

---

## Chapter 15 — Configuration

### 15.1 The file

All settings live in `kiva.conf`, in the home directory. It is created with default values on the first run. You can edit it with a text editor (while KivaDB is stopped) or, more conveniently, with the `config` command.

```ini
[storage]
default_workspace = C:\Users\you\AppData\Roaming\KivaDB\data
auto_compact = true
compact_threshold = 30
compression = none
sync_writes = false
snapshot_interval = 300
snapshot_every = 1000
cache_values = false
cache_size_mb = 256
max_memory_mb = 0

[workspaces]
shop = C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop

[network]
port = 2911
bind = 127.0.0.1

[security]
auth_required = false

[server]
threads = 8
max_clients = 1000

[http]
enabled = true
port = 8080
rate_limit = 600
cors = true
stream = true

[resp]
enabled = true
port = 6379
max_collection_members = 100000

[vector]
max_ram_mb = auto

[tls]
enabled = true
required = false
cert =
key =
hosts =

[encryption]
key_file =
old_key_file =
required = false

[replication]
role = master
master_host =
master_port = 2911
master_user = kiva
master_password =
master_tls = false
master_fingerprint =
sync_mode = async
min_slaves_ack = 1

[scripting]
timeout_ms = 5000
memory_limit_mb = 64
```

### 15.2 The `config` command

```text
kiva> config get compression
storage.compression = none
kiva> config set compact_threshold 25
OK: storage.compact_threshold = 25
kiva> config list
Configuration file: C:\Users\you\AppData\Roaming\KivaDB\kiva.conf
  storage.default_workspace        = C:\Users\you\AppData\Roaming\KivaDB\data
  storage.auto_compact             = true
  ...
```

When a name exists in several sections, write `section.key`:

```text
kiva> config get port
Error: 'port' exists in several sections; use one of: network.port, http.port
kiva> config get network.port
network.port = 2911
```

`config` requires the admin role. Values are checked (`config set compact_threshold abc` is refused). Passwords are shown as `***`.

### 15.3 Every setting

| Setting | Default | Meaning | Takes effect |
| :--- | :--- | :--- | :--- |
| `storage.default_workspace` | `<home>/data` | Folder of the `default` workspace | next start |
| `storage.auto_compact` | `true` | Compact automatically ([17.3](#173-compaction)) | immediately |
| `storage.compact_threshold` | `30` | Fragmentation percentage that triggers it | immediately |
| `storage.compression` | `none` | `none`, `lz4` or `zlib` for new writes ([17.5](#175-compression)) | immediately |
| `storage.sync_writes` | `false` | Every write on the physical disk before it is acknowledged, with group commit ([17.2](#172-what-survives-what)) | immediately |
| `storage.snapshot_interval` | `300` | Seconds between index snapshots | next start |
| `storage.snapshot_every` | `1000` | ...or after this many modifications | next start |
| `storage.cache_values` | `false` | Keep recently read values in memory ([Chapter 18](#chapter-18--keeping-values-in-memory-value-cache)) | immediately |
| `storage.cache_size_mb` | `256` | Memory budget of that cache, per workspace | immediately |
| `storage.max_memory_mb` | `0` | Memory bound per workspace: beyond it, new keys are refused ([18.6](#186-a-memory-bound)); `0` = none | immediately |
| `network.port` | `2911` | KivaDB protocol port | next start |
| `network.bind` | `127.0.0.1` | Address to listen on | next start |
| `security.auth_required` | `false` | Ask for a user name and password | immediately |
| `time.zone` | `local` | Time zone dates are shown and read in: `local`, `UTC`, `Africa/Kinshasa`, `+01:00` ([5.9](#59-time-zones)) | immediately |
| `server.threads` | `8` | Commands executed at the same time | next start |
| `server.max_clients` | `1000` | Simultaneous connections | immediately |
| `http.enabled` | `true` | Start the REST API | next start |
| `http.port` | `8080` | REST API port | next start |
| `http.rate_limit` | `600` | Requests per minute per client address, `0` = unlimited | immediately |
| `http.cors` | `true` | Allow browsers on other sites to call the API | immediately |
| `http.stream` | `true` | Offer `/v1/stream` (Server-Sent Events) | immediately |
| `resp.enabled` | `true` | Start the Redis-compatible listener ([Chapter 23](#chapter-23--redis-compatibility)) | next start |
| `resp.port` | `6379` | Its port | next start |
| `resp.max_collection_members` | `100000` | Members a Redis set, sorted set or list may hold ([23.8](#238-sets-and-sorted-sets)); `0` = no limit | immediately |
| `vector.max_ram_mb` | `auto` | Memory the vector indexes of a workspace may take: `auto` = half the RAM (or of the container's limit), `0` = no bound of their own ([12.10](#1210-memory-bounds)) | immediately |
| `tls.enabled` | `true` | Accept encrypted connections ([16.11](#1611-encrypted-connections-tls)) | next start |
| `tls.required` | `false` | Refuse plain connections from other machines | immediately |
| `tls.cert`, `tls.key` | empty | Your own certificate and key; empty: KivaDB's own | next start |
| `tls.hosts` | empty | Extra names for KivaDB's certificate | `kivadb tls renew` |
| `encryption.key_file` | empty | File holding the encryption key ([16.12](#1612-encryption-at-rest)) | next start |
| `encryption.old_key_file` | empty | Previous key, read during a key rotation | next start |
| `encryption.required` | `false` | Refuse to start without a key, and refuse unencrypted records | next start |
| `replication.*` | | See [Chapter 25](#chapter-25--replication) | |
| `semantic.cache.enabled` | `false` | Also cache the results of `get similar` ([12.8](#128-semantic-cache-reuse-answers-to-the-same-question)) | immediately |
| `semantic.cache.threshold` | `0.98` | Cosine similarity from which two questions count as the same | immediately |
| `semantic.cache.ttl` | `300` | Seconds a cached answer lives; `0` = until invalidated | immediately |
| `semantic.cache.max_entries` | `10000` | Answers kept per cache | immediately |
| `failover.auto` | `false` | Replace a dead master automatically ([25.8](#258-automatic-failover)) | immediately |
| `failover.heartbeat_ms` | `1000` | Milliseconds between the master's heartbeats | immediately |
| `failover.down_after_ms` | `5000` | Silence after which a replica considers the master down | immediately |
| `failover.quorum` | `0` | Nodes that must see the master down; `0` = a majority of the group | immediately |
| `failover.priority` | `100` | Preference to become master; `0` = never by itself | immediately |
| `scripting.timeout_ms` | `5000` | Longest a Lua script may run | immediately |
| `scripting.memory_limit_mb` | `64` | Most memory a Lua script may use | immediately |

---

## Chapter 16 — Users, roles and security

### 16.1 The starting point

A fresh KivaDB has one account, `kiva`, which is an administrator with an **empty password**, and authentication is **off**: anyone who can run `kivadb` on the machine is `kiva`. That is convenient on your own computer. Before sharing the machine or opening the network, secure it ([section 16.6](#166-turning-authentication-on)).

```text
kiva> whoami
kiva (admin) @ workspace 'default'
```

### 16.2 Roles

Every user has one role:

| Role | Can do |
| :--- | :--- |
| `admin` | Everything, in every workspace |
| `editor` | Read and write data, transactions, run scripts |
| `viewer` | Read only: `get`, `has`, `typeof`, `print`, listings, queries |

Administration commands (`compress`, `config`, `create`, `drop`, `grant`, `revoke`, `users`, `audit`, `replica`, `del all keys`, `shutdown`, `project create`, ...) need `admin`. One exception: an `editor` who can write in every workspace of a project manages it: they may run `compact` in its workspaces and create new ones in it ([16.4](#164-workspaces-and-grants)).

### 16.3 Managing accounts

A new account reaches **no workspace**: it can log in, but sees nothing until it is given access. Give it in the same command, or later with `grant` ([16.4](#164-workspaces-and-grants)):

```text
kiva> create user "fordi" identified by "mon_pass"
User 'fordi' created with role [editor]: no workspace yet (give it some with grant).
kiva> create user "app" identified by "app_pass" grant write on travel.* default workspace travel.places
User 'app' created with role [editor]: write travel.*.
kiva> create user "guest" identified by "guest_pass" role viewer grant read on travel.places, school.places
User 'guest' created with role [viewer]: read school.places, read travel.places.
kiva> users
 USER                 | ROLE    | CREATED             | GRANTS
------------------------------------------------------------------------------
 app                  | editor  | 2026-10-12 09:44:55 | travel.*=write (default: travel.places)
 fordi                | editor  | 2026-10-12 09:44:55 | none
 guest                | viewer  | 2026-10-12 09:44:55 | school.places=read, travel.places=read
 kiva                 | admin   | 2026-10-12 09:44:53 | all (admin)
```

```text
create user "<name>" identified by "<password>" [role admin|editor|viewer]
    [grant read|write on <scope>[, <scope>...]]...  [default workspace <name>]
```

A new user is an `editor` unless you say otherwise. `grant` may come several times (`grant write on travel.* grant read on default`). `default workspace` is where the user lands at login ([16.5](#165-logging-in)). User names may contain letters, digits, `_`, `-` and `.`.

Changing an account:

```text
kiva> modify user "fordi" password "new_pass"
Password updated for 'fordi'.
kiva> modify user "guest" role "editor"
Role of 'guest' set to [editor].
kiva> modify user "fordi" default workspace travel.places
Default workspace of 'fordi' set to 'travel.places'.
kiva> modify user "fordi" default workspace none
'fordi' has no default workspace any more.
kiva> drop user "guest"
User 'guest' dropped.
```

Anyone may change their own password; only an admin may change someone else's password or a role. The `kiva` account cannot be dropped or demoted.

### 16.4 Workspaces and grants

* An **admin** can use every workspace.
* Every other user reaches only what an admin **grants** them. A grant gives *read* or *write* access to a **scope**:

| Scope | Covers |
| :--- | :--- |
| `travel.places` | That workspace of project `travel` |
| `analytics`, `default` | That workspace, outside any project |
| `travel.*` | Every workspace of project `travel`, also those created later |
| `*.places` | The workspace `places` of every project |
| `*.*` (or `*`, or `all`) | Every workspace |

Several scopes go in one grant, separated by commas:

```text
kiva> grant write on travel.*, school.places to "fordi"
Granted: write on travel.*, school.places to 'fordi'.
kiva> grant read on *.places to "guest"
Granted: read on *.places to 'guest'.
kiva> grant write on *.* to "ops"
Granted: write on all workspaces to 'ops'.
```

| Command | Effect |
| :--- | :--- |
| `grant read on <scopes> to "<u>"` | The user may read those workspaces |
| `grant write on <scopes> to "<u>"` | The user may read and write them |
| `grant admin to "<u>"` | The user becomes an admin |
| `revoke write on <scopes> from "<u>"` | Write access there becomes read access |
| `revoke read on <scopes> from "<u>"` | Those grants are removed |
| `revoke write from "<u>"` | The user can no longer write anywhere (becomes a viewer) |
| `revoke read from "<u>"` | Every grant is removed |
| `revoke admin from "<u>"` | The user becomes an editor |

The older forms still work: `on workspace "shop"` for one workspace, `on all` for every one, and `on project travel` means `travel.*`. A grant names a workspace or a project that exists; a revoke may name one that is gone. A revoke removes the grant on exactly that scope: `revoke read on travel.places` does not cut into a grant on `travel.*`.

What a user can actually do in a workspace is the highest access their grants give there, bounded by their role: an `editor` with *read* access can only read there; a `viewer` with *write* access still cannot write. Granting *write* to a viewer turns them into an editor.

**Managing a project.** An `editor` with *write* on `travel.*` (or on `*.*`) manages project `travel`: they may create workspaces in it (`workspace create travel.logs`, in KivaDB's own folder) and run `compact` in its workspaces. Everything else that changes the server (creating projects, users, indexes, removing or moving workspaces) stays with admins. Outside a project, `compact` needs an admin, because it rewrites a whole shared file.

A user enters a workspace they were granted with `workspace switch <name>`; `workspace list` and `project list` show only what they can reach. Changes to grants apply the next time the user switches workspace or logs in.

**Accounts created before 2.0** had a personal workspace `u_<name>`, reached without any grant. On its first start, KivaDB 2.0 turns it into an ordinary grant (*write* on `u_<name>`) and makes it the account's default workspace: applications connected with those accounts keep landing on their data. To give it a clearer name, move it into a project ([14.7](#147-renaming-or-moving-a-workspace)): the grant and the default follow it.

### 16.5 Logging in

When authentication is on, the shell asks who you are:

```text
$ kivadb
Username [kiva]: fordi
Password: ********
Welcome fordi (editor)
kiva> whoami
fordi (editor) @ workspace 'travel.places'
```

After logging in, a user lands in, in this order: their **default workspace** (`modify user ... default workspace`), if they can still reach it; `default` for an administrator; the **only** workspace they can reach, if there is just one. Otherwise they land in **no workspace**: `whoami` says `fordi (editor), in no workspace`, commands on data answer `you are in no workspace`, and `workspace list` then `workspace switch <name>` choose one.

To land straight in a workspace, log in as `<user>@<workspace>`: `auth fordi@travel.cache mon_pass`, `kivadb -u fordi@travel.cache`, or the same user name over HTTP and Redis ([23.4](#234-logins-databases-roles)). The login fails if the user cannot reach that workspace.

Pressing Enter at the user name prompt chooses `kiva`, the built-in administrator. On the command line: `kivadb -u fordi` asks for the password; `kivadb -u fordi -p mon_pass` gives it directly (convenient in scripts, but the password is then visible in your command history).

Inside any session, `auth <user> <password>` logs in or switches user. This is also how you log in over a plain-text connection ([Chapter 20](#chapter-20--the-tcp-server)).

### 16.6 Turning authentication on

Do it in this order, so you do not lock yourself out:

```text
kiva> modify user "kiva" password "a-long-and-secret-password"
Password updated for 'kiva'.
kiva> config set auth_required true
OK: security.auth_required = true
```

From now on every new shell, connection and HTTP request must present a user name and password. As long as an account has an empty password, KivaDB reminds you at login:

```text
[WARN] kiva password is empty. Please run: modify user "kiva" password "..."
```

### 16.7 Opening KivaDB to the network

By default KivaDB only listens on `127.0.0.1`, which means only programs on the same computer can connect. To accept connections from other machines:

1. Turn authentication on and give `kiva` a password ([16.6](#166-turning-authentication-on)).
2. `config set bind 0.0.0.0` (all network interfaces) or the address of one interface.
3. Restart KivaDB (`kivadb stop`, `kivadb start`).

KivaDB **refuses to start** on a non-local address while authentication is off or `kiva` has no password:

```text
Error: refusing to listen on 0.0.0.0 without authentication.
```

Two more protections are always active: an account with an empty password can only log in from the local machine, and the REST API limits the number of requests per client ([21.7](#217-cors-and-rate-limiting)).

**Encryption.** On a network you do not fully control, encrypt the connections: KivaDB does it itself on every port, and `config set tls.required true` makes it mandatory for other machines ([16.11](#1611-encrypted-connections-tls)).

### 16.8 How passwords are stored

Never in clear. For each account KivaDB keeps a random *salt* and the result of PBKDF2-HMAC-SHA256 (60 000 rounds) applied to the password. Accounts are stored in KivaDB's own engine, in a system database inside the home directory (`system/system.kiva`).

### 16.9 The audit log

Sensitive actions are recorded in `audit.log` in the home directory: account and privilege changes, deletions, compaction, configuration changes, logins. Passwords are masked.

```text
kiva> audit last 3
[2026-10-12 09:44:55] user=kiva action="create user 'fordi' identified by '***'" status=SUCCESS
[2026-10-12 09:44:55] user=kiva action="create user 'guest' identified by '***' role viewer" status=SUCCESS
[2026-10-12 09:44:55] user=kiva action="grant read on workspace 'shop' to 'guest'" status=SUCCESS
```

| Command | Shows |
| :--- | :--- |
| `audit last <N>` | The N most recent entries |
| `audit since "2026-10-01"` | Entries from that date (or `"2026-10-01 14:00:00"`) |
| `audit failed-logins` | Rejected login attempts |

### 16.10 Emergency access

If you lose the kiva password, someone with access to the machine can start KivaDB without authentication, fix the account, and restart normally:

```bash
kivadb stop
kivadb --skip-auth
```

```text
[WARN] Authentication disabled - unsafe for production
kiva> modify user "kiva" password "new-password"
```

### 16.11 Encrypted connections (TLS)

Passwords are never stored in clear ([16.8](#168-how-passwords-are-stored)), but without encryption they, and the data, cross the network in clear. TLS, the encryption of HTTPS, protects them on the way.

**Nothing to install, no new port.** Every KivaDB port accepts encrypted connections next to plain ones: 2911 (shells, Kiva Studio, drivers), 8080 (which then answers `https://` as well as `http://`) and 6379 (`rediss://` for Redis clients). KivaDB tells them apart by the first byte the client sends.

**The certificate.** On its first start, KivaDB creates its own certificate in `<home>/tls/` (`cert.pem`, and the private key `key.pem`, readable by its owner only). `kivadb tls` shows it:

```bash
kivadb tls
```

```text
Certificate : /root/.kivadb/tls/cert.pem
SHA-256     : 77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6
Encryption  : accepted; plain connections are accepted too (config set tls.required true to refuse them)
Clients check this fingerprint the first time they connect.
```

No authority vouches for this certificate (a server known by its IP address cannot get one from Let's Encrypt), so clients check its **fingerprint** instead, as SSH does: the first time, they show it and ask whether to trust it; after that they remember it, and refuse to go on if it ever changes. Compare it once with what `kivadb tls` shows on the server.

**Requiring encryption.** By default plain connections are still accepted, so that nothing breaks. Once your clients use TLS, refuse the others:

```text
kiva> config set tls.required true
```

Connections from other machines must then be encrypted; the machine itself (`127.0.0.1`, where `kivadb stop` and local applications connect) may still use plain connections. A refused client gets a clear message in its own protocol:

```text
Error: this server requires an encrypted connection (TLS) from other machines
```

**Connecting with encryption:**

| Client | How |
| :--- | :--- |
| Shell | `kivadb --connect 31.25.236.110:2911 --tls -u kiva`. The first time, it shows the fingerprint and asks; trusted servers are kept in `<home>/known_servers`. In scripts: `--tls-fingerprint <fingerprint>` |
| Kiva Studio | The switch *Encrypt the connection (TLS)*, on by itself for a remote address. A window shows the fingerprint the first time |
| Node.js driver | `connect({ host, password, tls: { fingerprint: "77:45:..." } })` |
| Rust client | `Client::connect_tls("host:2911", Some("77:45:..."))` |
| Redis libraries | `rediss://user:password@host:6379` with the certificate as authority, e.g. ioredis `tls: { ca: fs.readFileSync("cert.pem") }`; copy `cert.pem` (never `key.pem`) to the application's machine |
| curl, browsers | `curl --cacert cert.pem https://host:8080/health`; a browser warns about the certificate unless it is installed or you use your own (below) |
| Replicas | `config set master_tls true` and `config set master_fingerprint "<the master's fingerprint>"` on the replica |

For clients that check names (Redis libraries, curl, browsers), the certificate must contain the name or address they connect to. The one KivaDB creates covers `localhost`, `127.0.0.1` and the machine's name; add the server's public address, then create a new certificate and restart:

```bash
kivadb -u kiva -e "config set tls.hosts 31.25.236.110"
kivadb tls renew
kivadb stop
kivadb start
```

After `renew`, clients see a new fingerprint: they must trust it again (in Kiva Studio, forget the server in the recent connections first).

**Your own certificate.** With a domain name, a certificate from Let's Encrypt (or any authority) is checked by clients without any fingerprint. Point KivaDB to its files:

```text
kiva> config set tls.cert "/etc/letsencrypt/live/db.example.com/fullchain.pem"
kiva> config set tls.key "/etc/letsencrypt/live/db.example.com/privkey.pem"
```

| Setting | Default | Meaning |
| :--- | :--- | :--- |
| `tls.enabled` | `true` | Accept encrypted connections |
| `tls.required` | `false` | Refuse plain connections from other machines |
| `tls.cert`, `tls.key` | empty | Your certificate and key (PEM); empty: the one KivaDB creates |
| `tls.hosts` | empty | Extra names or addresses for the certificate KivaDB creates |

### 16.12 Encryption at rest

TLS protects data while it travels. **Encryption at rest** protects it where it sleeps: a stolen disk, a copied backup or a cloud snapshot of the server shows only random bytes, not your keys and values. KivaDB encrypts with **AES-256-GCM**, the algorithm of disk and cloud encryption, through the `ring` library already used for TLS.

**1. Create a key.**

```bash
kivadb keygen
```

It prints 64 hexadecimal characters: 32 random bytes. **Keep this key apart from the data and keep a copy in a safe place** (a password manager, a vault). Without it, encrypted data cannot be read by anyone, KivaDB included: there is no recovery.

**2. Give it to KivaDB**, in one of two ways:

* the environment variable `KIVADB_ENCRYPTION_KEY`:

  ```bash
  export KIVADB_ENCRYPTION_KEY=3f9c...   # the 64 characters
  kivadb start
  ```

* or a file holding it, readable only by the account running KivaDB, named in `kiva.conf`:

  ```bash
  kivadb keygen > /etc/kivadb/key && chmod 600 /etc/kivadb/key
  kivadb -e "config set encryption.key_file /etc/kivadb/key"
  ```

The key itself is never written in `kiva.conf` nor anywhere in the data folder. KivaDB reads it when it starts; a change of key file needs a restart. With systemd, put the variable in a file readable by root only and load it with `EnvironmentFile=`; with Docker, pass `-e KIVADB_ENCRYPTION_KEY` or mount the key file as a secret.

**3. Check.**

```text
kiva> encryption
Encryption at rest: on (AES-256-GCM, key 5d1e07a2)
Workspace 'default': 1250 records in the data file, 1250 encrypted, 0 in clear
 - every record is encrypted with the current key
Index snapshot (hint): encrypted
```

The key id (`5d1e07a2`) identifies the key without revealing it.

**What is encrypted.** In every workspace and in the system database (users, password hashes, stored scripts, triggers):

* each record of the data file: its key and its value, together. The record's header (sizes, type, dates of writing and expiry) stays readable, so that KivaDB can find its way through the file, but it is authenticated with the record: changing any byte of a record, even with its checksum fixed, makes it rejected instead of read;
* the index snapshot (`.hint`), which lists every key;
* the HNSW graphs (`.graphs`), which hold keys and vectors;
* copies made by KivaDB (`.bak`, the temporary file of a compaction), which are copies of encrypted records.

Every encryption uses a fresh random number (nonce): the same value written twice never gives the same bytes.

**What stays in clear**, by design:

* `kiva.conf`, the index definitions (`.meta`: patterns and field names), the TLS certificate;
* `audit.log` and `server.log`: user names, actions, and the names of deleted keys. They are meant to be read and forwarded;
* the memory of the running server (values in use, the value cache) and anything the operating system writes from it (swap, crash dumps): use the system's disk encryption for those;
* on a client computer, the shell history of `kivadb --connect`. On the server itself, the shell does not keep a history file when encryption is on;
* what leaves through the protocol: replicas receive the data and encrypt it with their own key (give them one, and use TLS between them); exports you make yourself are yours to protect.

**Data written before the key.** Turning encryption on does not rewrite the past: old records stay in clear and readable, new ones are encrypted. `compact` rewrites the whole file with the key, old versions included; `encryption` then shows `0 in clear`. Do it in every workspace (`workspace switch`, then `compact`).

**`encryption.required true`** turns the protection into a rule: KivaDB refuses to start without a key, and refuses a data file holding records in clear (someone could otherwise slip readable records into it). Turn it on once every workspace is compacted.

**Changing the key (rotation).** Give the new key as usual, and the old one as `KIVADB_ENCRYPTION_OLD_KEY` (or `encryption.old_key_file`); restart. Both are read, new writes use the new key. Run `compact` in each workspace: `encryption` no longer counts records `encrypted with an older key`. The old key can then be removed. Rotate when a key may have been exposed, when someone who knew it leaves, and, as good practice, every year or so. (AES-GCM with random nonces stays safe for some four billion writes per key; a yearly rotation keeps far below.)

**Cost.** 32 bytes per record. In our measurements (`bench write` / `bench read` of 100 000 keys), writes were 10 to 15 % slower; reads were within the variation between two runs.

**Errors.** `this workspace is encrypted: start KivaDB with its key`: no key was given. `encrypted with key 5d1e07a2, which this server does not have`: the key given is another one. In both cases nothing is changed on disk; start again with the right key.

---

## Chapter 17 — Maintenance and reliability

### 17.1 `stats` — the dashboard

```text
kiva> stats

==========================================================
     KivaDB Engine Statistics
----------------------------------------------------------
 > Workspace            : default
 > Total Keys           : 11 (1 with TTL)
 > Storage Size         : 1.12 KB
 > Live Data Size       : 639 bytes
 > Disk Fragmentation   : 44.4% [Action Needed: 'compact']
 > Memory Usage (RAM)   : 1.82 KB
 > Compression          : none (254 bytes -> 254 bytes, 0.0% saved, 0 records)
 > Startup              : 4.12 ms (new database)
 > Checksum Errors      : 0
 > Last Integrity Check : never
 > Auto-Compaction      : on (threshold 30%), 0 run(s)
 > Secondary Indexes    : 1 (+ 0 vector)
 > Value Cache          : off (values are read from disk)
 > Replication          : master, 0 replica(s), max lag 0 ops
 > Database Path        : C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva
==========================================================
```

| Line | Meaning |
| :--- | :--- |
| Storage Size | Size of the data file on disk |
| Live Data Size | The part of it that holds current values |
| Disk Fragmentation | The share that is dead weight (old versions, deleted keys) |
| Memory Usage | Estimated RAM used by the index |
| Compression | Size of values before and after compression |
| Startup | How long loading took, and how (`new database`, `from hint file`, `full log scan`) |
| Checksum Errors | Damaged records detected since start |
| Last Integrity Check | When `check` last ran and what it found |
| Value Cache | Memory used by cached values and the share of reads it answered ([Chapter 18](#chapter-18--keeping-values-in-memory-value-cache)) |

### 17.2 What survives what

| Event | Result |
| :--- | :--- |
| The shell or server is closed normally | Nothing is lost |
| The process crashes or is killed | Nothing is lost. A write that was cut in half is discarded at the next start |
| Power failure or operating system crash | By default, up to the last second of writes can be lost. With `sync_writes = true`, nothing is |
| A transaction was open | It is as if it never started |
| A compaction was running | The original file is intact; the unfinished copy is thrown away |

`sync_writes = true` asks the disk to confirm every write before KivaDB answers it: an acknowledged write survives a power failure. It applies immediately (`config set sync_writes true`). The default (confirm once per second) is the usual compromise.

**Group commit.** A disk confirmation (fsync) takes about a millisecond on an SSD, often more on Windows or on a hard disk. With `sync_writes`, KivaDB does not make each client wait for its own: the writes that arrive while one fsync is under way are confirmed together by the next one. A single client writes at the speed of the disk, but many clients writing at once share the confirmations, so the total number of writes per second grows with the number of clients instead of staying at one fsync each. Every write is still on disk before it is acknowledged.

* One writer at a time runs the fsync, outside the workspace lock: the others keep appending to the file meanwhile, then wait for its end. The ones it covered return together; one of the others runs the next fsync for all of them.
* Before syncing, it lets the writes already on their way finish appending, for up to a millisecond, so that they share it.
* Redis clients ([Chapter 23](#chapter-23--redis-compatibility)) run their commands one at a time, but wait for the disk after their command: the others go on, and their writes join the next fsync.

Measured on Windows with an SSD (release build, writes of 100 bytes):

| Writers | Writes per second | Writes per fsync |
| :--- | :--- | :--- |
| 1 thread | 800 | 1 |
| 16 threads | 8 400 (690 before) | 12 (3 before) |
| 64 threads | 20 000 (880 before) | 52 (3 before) |
| 16 Redis clients | about 5 000 (216 before) | |

The tests `group_commit_speed` (`tests/engine.rs`) and `tests/redis-clients/sync-bench.mjs` repeat these measures. `stats` shows how well it works:

```text
 > Durability           : on disk before each answer (12000 writes, 2210 disk syncs, 5.4 writes per sync)
```

Other clients can read a write a few milliseconds before its fsync ends; only its own client is answered after it.

After an unclean stop, the shell tells you what it did:

```text
[INFO] Recovered after an unclean shutdown: 37 bytes of incomplete log discarded.
```

### 17.3 Compaction

KivaDB never modifies data in place. An `update` appends the new value at the end of the file; a `del` appends a marker saying the key is gone. This is fast and safe, but the old versions stay in the file. **Compaction** rewrites the file with only what is current.

```text
kiva> compact
Database storage compacted successfully (1.12 KB -> 639 bytes, 11 records).
```

Compaction does not stop the database: reads and writes continue while the bulk of the copy happens. `compact` needs the admin role, or, in a workspace of a project, write access to every workspace of that project ([16.4](#164-workspaces-and-grants)).

**Automatic compaction** does this for you in the background when fragmentation reaches a threshold (30 % by default) on a file of at least 64 KB:

```text
kiva> autocompact status
Auto-compaction: ON
  Threshold: 30% fragmentation
  Current fragmentation: 0.0%
  Compactions run: 2
kiva> autocompact threshold 25
Auto-compaction is ON (threshold 25%).
kiva> autocompact off
Auto-compaction is OFF (threshold 25%).
```

### 17.4 Integrity: `check`

Every record on disk carries a checksum (CRC32). Whenever a value is read, the checksum is verified; a damaged record is never returned as if it were valid:

```text
kiva> get victim
Error: ERR_DATA_CORRUPTED: checksum mismatch on key 'victim'
```

`check` verifies everything at once:

```text
kiva> check
Checking 11 entries...
  OK: 11
  CORRUPTED: 0
Done.
```

If some records are damaged (failing disk, file modified by another program), `check repair` saves a copy of the data file next to it (`store.kiva.bak`) and removes the unreadable entries:

```text
kiva> check repair
Checking 3 entries...
  OK: 2
  CORRUPTED: 1
    - victim
  Backup saved to C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva.bak
  Repairing... 1 entries removed.
Done.
```

A removed entry cannot be recovered by KivaDB; restore it from your own backup or from a replica.

### 17.5 Compression

Values of 64 bytes or more can be compressed before being written.

| Algorithm | Character |
| :--- | :--- |
| `lz4` | Very fast, good ratio. The usual choice |
| `zlib` | Smaller files, slower |
| `none` | No compression |

```text
kiva> compress lz4
Database re-encoded with compression 'lz4': 12.40 MB -> 3.10 MB on disk (50000 records, values 76.2% smaller).
```

(The figures above are an illustration; yours depend on your data.)

```text
kiva> config get compression
storage.compression = lz4
```

`compress` does two things: it rewrites the whole current workspace with the chosen algorithm, and it makes that algorithm the setting for all future writes (`storage.compression`). Reading is transparent: `get` returns the original value whatever the compression. A file can contain records compressed in different ways; `scan` shows the algorithm of each key in the STATUS column. Values that would not shrink are stored as they are.

### 17.6 The hint file

To answer quickly, KivaDB keeps in memory an **index**: for each key, where its value is in the data file. Rebuilding that index by reading the whole data file would be slow for a large database, so KivaDB regularly saves a compact copy of the index in a **hint file**. At start it loads the hint file, then reads only what was written after it.

```text
kiva> hint info
Hint File: C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva.hint
  Status: VALID
  Entries: 11
  Size on disk: 545 bytes
  Compression: none
  Last updated: 2026-10-12 09:44:54
  Checksum: PASSED
```

The hint file is written at every clean stop, after every compaction, and periodically (`snapshot_interval`, `snapshot_every`). It is protected by checksums; if it is missing or damaged, KivaDB simply reads the data file in full. Deleting it is harmless.

### 17.7 Backups

**Saving.** One command saves everything (configuration, users and their passwords, scripts, every workspace wherever its folder is, the audit log, the TLS certificate) into one `.tar.gz` file:

```bash
kivadb backup --output /backups/kivadb-2026-10-22.tar.gz
```

```text
Backup written: /backups/kivadb-2026-10-22.tar.gz (12.4 MB, 9 file(s), workspace(s): default, shop)
SHA-256: 3b0c...e91f (also in /backups/kivadb-2026-10-22.tar.gz.sha256)
```

* **KivaDB may keep running.** When it runs, the command asks the server to make the backup (give `-u kiva -p ...` when authentication is on); otherwise it opens the files itself. Each data file is saved as it was at one instant: writes made during the backup simply come after it, and a compaction waits until the copy is done. Nothing is paused.
* `-w shop` saves one workspace only.
* Next to the archive, `<archive>.sha256` holds its SHA-256, in the format of `sha256sum -c`. Inside, a manifest lists every file with its own SHA-256.
* The index snapshot and the HNSW graphs are not saved: KivaDB rebuilds them. Encrypted data stays encrypted in the archive ([16.12](#1612-encryption-at-rest)): keep the key apart from the backups, and do not lose it.
* An existing file is never overwritten: give each backup its own name, for example with the date. From the shell, `backup to "/backups/x.tar.gz"` does the same (admin; the path is on the server).

**Restoring.** KivaDB must be stopped on the home that receives the data. Nothing is written until the whole archive has been checked: its SHA-256 against the `.sha256` file, every file against the manifest, and the checksum of every record.

```bash
kivadb restore /backups/kivadb-2026-10-22.tar.gz --check                    # only check
kivadb restore /backups/kivadb-2026-10-22.tar.gz --target /srv/kivadb-new   # a whole home
```

* **A home backup** is restored into an empty folder (`--target`, or the home of the command if it is empty); its workspaces are placed inside it (`data`, `workspaces/<name>`) and `kiva.conf` is updated to point at them. Then start it: `kivadb --home /srv/kivadb-new start`. To replace a damaged home, move it away first, then restore in its place.
* **A workspace backup** becomes a workspace of the current home, under its own name or another one: `kivadb restore shop.tar.gz --as shop_old` (`--target` chooses its folder). An existing workspace is never overwritten.
* A damaged archive is refused with the reason (`its SHA-256 does not match`, `store.kiva does not match its SHA-256`, `checksum mismatch in the record at byte ...`), and nothing changes.

**Keep them safe.** Copy the archives to another machine or another site: a backup on the same disk dies with it. Restore one now and then, as a test (29.7). For a copy that stays up to date continuously, use a replica ([Chapter 25](#chapter-25--replication)); a replica does not replace backups, since a deletion reaches it too.

### 17.8 Measuring speed

```text
kiva> bench write 100000
WRITE: 100000 ops in 3.514s -> 28459 ops/sec (avg 0.035 ms)
kiva> bench read 100000
READ : 100000 ops in 0.973s -> 102739 ops/sec (avg 0.010 ms)
```

`bench` writes temporary keys (`__bench:...`) in the current workspace and removes them afterwards. The figures depend on your disk and processor.

### 17.9 `kivadb doctor` — a health check of the home

`kivadb doctor` looks for what could stop KivaDB from starting, from being reached safely or from keeping its data. It only reads: it works whether KivaDB is running or not, needs no password, and changes nothing in the home.

```text
$ kivadb doctor
KivaDB 2.0.0 doctor, home /home/ana/.local/share/kivadb

  OK    Home        /home/ana/.local/share/kivadb is writable
  OK    Disk        41.20 GB free, data takes 12.07 MB
  OK    Server      running (pid 4120) on port 2911
  FAIL  Access      KivaDB listens on 0.0.0.0, open to other machines, without asking for a password
                    -> config set security.auth_required true, after giving kiva a password (modify user "kiva" password ...)
  OK    TLS         on, certificate /home/ana/.local/share/kivadb/tls/cert.pem (SHA-256 4f:9a:...)
  OK    Encryption  off (data files are not encrypted)
  OK    Time zone   Africa/Kinshasa, now 2026-10-27T13:50:40.759Z
  OK    Replication master
  OK    Data        3 file(s), 18204 record(s), every checksum matches
  OK    Server log  no recent error

9 check(s) passed, 0 warnings, 1 problem.
```

| Line | What is checked |
| :--- | :--- |
| Home | The folder can be written by this user |
| Disk | Free space: below 1 GB, or below the size of the data, a compaction (which rewrites the live data) may fail |
| Server | Whether KivaDB runs on this home; when it does not, whether its ports (TCP, REST, Redis) are free |
| Access | A listening address open to other machines without `security.auth_required` is a problem |
| TLS | The certificate can be loaded; open to the network without `tls.required` is a warning |
| Encryption | The key is given when the data needs one ([16.12](#1612-encryption-at-rest)) |
| Time zone | `time.zone` is a zone KivaDB knows ([5.9](#59-time-zones)) |
| Replication | A replica has a master, reached with TLS when it is on another machine |
| Data | Every record of every workspace and of `system`, against its checksum, read from the files |
| Server log | Errors among the last 500 lines of `server.log` |

A last record cut short is a warning, not a problem: a write was interrupted by a crash or a power cut, and KivaDB drops it the next time it opens the file. A checksum that does not match is a problem: `check` and `check repair` ([17.4](#174-integrity-check)) deal with it.

`kivadb doctor --json` gives the same findings to programs: `level` (`ok`, `warning` or `problem`), `topic`, `message` and `advice`, and the counts in `summary`. The exit code is 0 when nothing is a problem (warnings allowed) and 1 otherwise, so that a deployment script or a monitoring check can stop on it:

```bash
kivadb doctor --json > doctor.json || echo "KivaDB needs attention"
```

---

## Chapter 18 — Keeping values in memory (value cache)

### 18.1 Memory and disk in KivaDB

KivaDB uses both, for different things:

| What | Where | Why |
| :--- | :--- | :--- |
| Every value | On disk, in the data file | It survives a crash or a restart |
| Every key, with the position of its value | In memory (RAM) | Finding a key never touches the disk |
| Recently read values (optional) | In memory, within a budget | Reading them again never touches the disk |

So by default a read is one lookup in memory and one read from the data file. That read is usually served by the operating system's own file cache, which is fast but not free. For data read again and again - sessions, configuration, popular product pages - the **value cache** keeps the values themselves in memory as well.

### 18.2 Turning it on

```text
kiva> config set cache_values true
OK: storage.cache_values = true
kiva> config set cache_size_mb 512
OK: storage.cache_size_mb = 512
```

It takes effect immediately and can be turned off at any time (`config set cache_values false`). Both settings are in `kiva.conf` ([15.3](#153-every-setting)).

### 18.3 How it behaves

* The budget is per workspace. When it is full, the values read least recently are dropped.
* The file stays the truth: the cache only holds copies, checked against the version of each record, so it can never return an old value. A write, a delete or a transaction replaces or removes the copy.
* Durability is unchanged: writes go to disk exactly as before. This is a read cache, not a memory-only database.

### 18.4 Watching it

`stats` shows how it is doing:

```text
 > Value Cache          : 160 bytes of 256.00 MB used, 1 values, 50.0% hits
```

A high hit rate means the cache is useful; a low one on a large database means the reads are spread over too many keys for the budget. Raise `cache_size_mb` if the machine has memory to spare, or leave the cache off: KivaDB is fast without it.

### 18.5 Pinning keys in memory

Some keys must always answer from memory: configuration, feature flags, sessions, a price list. **Pin** them:

```text
kiva> set conf:a {"currency": "CDF", "vat": 16} and conf:b "on"
OK: conf:a saved.
OK: conf:b saved.
kiva> memory pin "conf:*"
Pinned 'conf:*': 2 values (233 bytes) kept in memory, now and after every write.
```

* The values of the keys matching the pattern are read into memory at once, and kept there: a write stores the new value in memory too, a delete removes it, and new keys matching the pattern are added.
* Pinned values are never pushed out, whether the value cache is on or off, and do not count against `cache_size_mb`.
* Pinned patterns are saved with the workspace; at start, their values are read into memory again.
* `memory unpin "conf:*"` lets them go back to being read from disk. Pinning and unpinning need the admin role.

Pin small, hot sets of keys. Everything pinned stays in RAM, so a pattern matching millions of large documents would fill it.

### 18.6 A memory bound

`memory` shows where a workspace's memory goes (estimates):

```text
kiva> memory
Memory of workspace 'default' (estimates)
  Keys              : 379 bytes (3 keys)
  Secondary indexes : 0 bytes
  Vector indexes    : 0 bytes of 8.00 GB (0.0%, vector.max_ram_mb)
  Pinned values     : 131 bytes (1 values)
  Value cache       : 0 bytes (given back first)
  Total             : 510 bytes
  Bound             : 1.00 MB (0.0% used)
  Pinned patterns   : order:*
```

Every key always lives in memory ([18.1](#181-memory-and-disk-in-kivadb)), with the indexes and the pinned values. To make sure a workspace never takes more than the machine can give, set a bound:

```text
kiva> config set max_memory_mb 512
OK: storage.max_memory_mb = 512
```

When the keys, indexes and pinned values reach it:

* the value cache gives its memory back first;
* then **new keys are refused**, with an error starting with `OOM:`. Changing a key that exists, reading and deleting still work, so you can make room;
* pinning a pattern whose values would not fit is refused too.

```text
kiva> set brand:new 1
Error: OOM: this workspace reached its memory bound (1023.99 KB of 1.00 MB used), so new keys are refused. Delete keys or expired data, drop an index, or raise storage.max_memory_mb.
```

The bound applies to each workspace and takes effect immediately; `0` (the default) means no bound. A replica always applies what its master sends, whatever its own bound: give replicas the same bound as the master, or none.

---

## Chapter 19 — How data is stored

You do not need this chapter to use KivaDB. It is here for the curious, and for anyone who has to understand the files.

### 19.1 The home directory

```text
KivaDB/
  kiva.conf            configuration
  audit.log            audit journal
  server.log           output of 'kivadb start'
  instance.lock        held by the running KivaDB process
  instance.port        its TCP port (removed when it stops)
  instance.pid         its process id (removed when it stops)
  history.txt          shell history
  version_check.json   result of the last check for a new release
  system/
    system.kiva        users, stored scripts, triggers
  data/                the 'default' workspace
    store.kiva
    store.kiva.hint
    store.kiva.meta
    store.kiva.lock
  workspaces/
    shop/              another workspace
      store.kiva
      ...
```

### 19.2 The files of a workspace

| File | Role | If deleted |
| :--- | :--- | :--- |
| `store.kiva` | The data | The data is lost |
| `store.kiva.hint` | Index snapshot for fast start | Rebuilt; the next start is slower |
| `store.kiva.meta` | Definitions of the secondary indexes, vector indexes and schemas, and the settings of the change journal | Indexes and schemas must be created again. A damaged file stops the opening of the workspace rather than open it without them ([30.1](#301-starting)) |
| `store.kiva.lock` | Prevents two processes from opening the files | Recreated |
| `store.kiva.bak` | Copy made before a repair | - |

### 19.3 The principle: an append-only log

`store.kiva` is a **log**: a sequence of records, each one added after the previous. Nothing already written is ever changed.

```text
[header][record: set a=1][record: set b=2][record: set a=5][record: delete b] ...
```

To know the current state, read the log from the start and keep, for each key, the last thing said about it. KivaDB does that once, at start, and keeps the result in memory as a sorted index (key → position of its latest record). After that:

* a **read** is one lookup in memory and one read on disk;
* a **write** is one append at the end of the file and one update in memory.

This design is simple, fast for writes, and safe: a crash can only damage the very last record, and that is detected.

### 19.4 The data file, byte by byte

All integers are little-endian.

**File header (16 bytes)**

| Field | Size | Content |
| :--- | :--- | :--- |
| magic | 4 | The letters `KIVA` |
| format | 4 | Storage format number: `1`, or `2` once the file holds a `date`, `datetime` or `time` |
| file_id | 8 | A random number, changed at every compaction and reset |

A file is raised from format 1 to format 2 the first time a `date`, `datetime` or `time` is written to it, just before that record. KivaDB 1.x reads only format 1: it refuses such a file with `unsupported storage format 2` instead of misreading the new records. A workspace that holds none of them stays at format 1, readable by 1.x. The values of these types are stored as text that sorts in time order: `2026-10-27`, `2026-10-27T14:30:00.000Z` (always UTC, to the millisecond), `08:00:00`.

**Record (28-byte header, then the key, then the value)**

| Field | Type | Size | Content |
| :--- | :--- | :--- | :--- |
| crc32 | `u32` | 4 | Checksum of all the following bytes of the record |
| k_size | `u16` | 2 | Length of the key |
| v_size | `u32` | 4 | Length of the value as stored |
| type | `u8` | 1 | 1 string, 2 number, 3 boolean, 4 null, 5 array, 6 object, 7 bytes, 8 date, 9 datetime, 10 time |
| flags | `u8` | 1 | See below |
| timestamp | `i64` | 8 | When it was written (seconds since 1970) |
| expires_at | `i64` | 8 | When it expires, Unix time in milliseconds; 0 = never. Files written by KivaDB 1.5.1 and earlier hold seconds: values below 10^11 are read as seconds |
| key | bytes | k_size | The key, UTF-8 |
| value | bytes | v_size | The value |

**Flags**

| Bits | Meaning |
| :--- | :--- |
| 0-1 | Compression: 0 none, 1 LZ4, 2 Zlib |
| 2 | The document is encoded in MessagePack instead of JSON |
| 5 | The record belongs to a batch (transaction) |
| 6 | The record is the last of its batch: the commit point |
| 7 | Tombstone: the key was deleted |

**How values are encoded**

| Type | Bytes |
| :--- | :--- |
| string | The UTF-8 text |
| number | The number as text (`3.14`) |
| boolean | `true` or `false` |
| null | Nothing (length 0) |
| array, object | JSON text, or MessagePack if flag bit 2 is set |
| bytes | The raw bytes |

A compressed value starts with its original length (`u32`), followed by the compressed data.

**Transactions.** The records of a transaction all carry the batch flag, and the last one also carries the commit flag. When reading the log, KivaDB sets batch records aside and applies them only when it meets the commit record. A batch without its commit record, which can only be at the very end of the file after a crash, is discarded.

### 19.5 The hint file

Header (44 bytes): the letters `KHNT`, a version number, the `file_id` of the data file it belongs to, the length of the data file it covers, the number of entries, the compression used, a checksum of the body and a checksum of the header.

Body: one entry per key - key, position in the data file, sizes, type, flags, timestamps - each followed by its own CRC32. The body is compressed with the workspace's algorithm.

The `file_id` is what makes the hint safe: if the data file was replaced (compaction, reset), the ids no longer match and the hint is ignored.

### 19.6 Limits

| Item | Limit |
| :--- | :--- |
| Key length | 1024 bytes |
| Value size | 256 MB |
| Number of keys | Bounded by memory: every key and about 100 bytes of bookkeeping live in RAM |
| Data file size | Bounded by the disk |

Values are **not** kept in memory, only keys and positions (unless the value cache of [Chapter 18](#chapter-18--keeping-values-in-memory-value-cache) is turned on, within its budget). A database can therefore be much larger than the RAM, as long as the keys fit.

---

## Chapter 20 — The TCP server

When KivaDB is running, it listens on a TCP port, **2911** by default. That one port understands two languages.

### 20.1 Plain text

Send a command, a line break, and read the answer. It is exactly the shell language. Any tool that opens a TCP connection can use it:

```bash
telnet localhost 2911
```

```text
set user:9 "Alice"
OK: user:9 saved.
get user:9
user:9: Alice [string]
exit
```

With authentication on, start with `auth`:

```text
get user:9
Error: permission denied: authentication required. Use: auth <user> <password>
auth fordi mon_pass
Welcome fordi (editor)
```

Text mode is ideal for trying things and for simple scripts. Its weakness: an answer can have any number of lines and nothing marks its end, so a program cannot reliably tell where one answer stops. Programs should use the binary protocol or the REST API.

### 20.2 The binary protocol

Every message, in both directions, is a **frame**:

```text
+----------------+-------------+------------------+
| length (u32)   | code (u16)  | payload          |
+----------------+-------------+------------------+
```

* Integers are big-endian (network order).
* `length` counts the code and the payload, that is, payload size + 2.
* In a request, `code` is a command number; in a reply, a status.

KivaDB decides which language a connection speaks from its very first byte: `0x00` means binary (the first byte of a small frame length is always zero), anything else means text.

**Requests**

| Command | Code | Payload | Reply payload |
| :--- | :--- | :--- | :--- |
| HELLO | 1 | empty | Three lines: `KivaDB 1.7.1`, `auth=required` or `auth=none`, `user=<name>` |
| AUTH | 2 | user, a zero byte, password | A welcome text |
| EXEC | 3 | A command line, as in the shell | Output lines separated by `\n`; each line starts with `+` (normal) or `-` (error) |
| PING | 4 | empty | `PONG` |
| GET | 5 | The key | One type byte, then the value bytes (see [19.4](#194-the-data-file-byte-by-byte); documents are JSON text) |
| SET | 6 | type `u8`, ttl `u32`, key length `u16`, key, value bytes | `OK` |
| DEL | 7 | The key | `1` |
| API | 8 | A JSON object with an `op` field | A JSON object (see [20.4](#204-structured-requests-for-tools)) |

SET creates the key or replaces it, whatever its previous type. A ttl of 0 means no expiry.

**Reply statuses**

| Status | Meaning |
| :--- | :--- |
| 0 | Success |
| 1 | Error; the payload is the message (for EXEC: at least one line starts with `-`) |
| 2 | Key not found (GET, DEL) |
| 3 | The server is closing the connection (after `exit` or `shutdown`) |
| 4 | The command was `clear` |
| 9 | A change notification ([Chapter 22](#chapter-22--change-notifications)); not a reply to a request |

**Example.** The request `EXEC get age` is these 13 bytes:

```text
00 00 00 09    length = 9 (2 for the code + 7 for the payload)
00 03          code = 3 (EXEC)
67 65 74 20 61 67 65    "get age"
```

and the reply:

```text
00 00 00 13    length = 19 (2 for the status + 17 for the payload)
00 00          status = 0
2B 61 67 65 3A 20 39 39 20 5B 6E 75 6D 62 65 72 5D    "+age: 99 [number]"
```

A connection stays open for as many requests as you like. Replies come back in the order of the requests. Ready-made clients are in [Chapter 24](#chapter-24--connecting-from-your-programs).

### 20.3 Capacity

Each connection is served by its own thread. `server.max_clients` (1000) caps the number of connections; `server.threads` (8) caps how many commands execute at the same instant.

### 20.4 Structured requests for tools

The command language answers with text written for people. A program that needs data it can rely on - a graphical client, a dashboard - uses the API request (code 8): a JSON object goes in, a JSON object comes back. The same session, permissions and transaction apply as for commands.

| `op` | Other fields | Answer |
| :--- | :--- | :--- |
| `whoami` | - | `user`, `role`, `workspace`, `access`, `can_write`, `can_compact` (whether `compact` is allowed in this workspace), `auth_required`, `replica`, `version` |
| `workspaces` | - | `workspaces`: name, path, access and whether it is the current one |
| `scan` | `pattern` (default `*`), `limit` (default 500) | `keys`: key, type, size, modified, ttl, compression, msgpack; `truncated`; `total` |
| `get` | `key` | `found`, `type`, `value`, `ttl`, `modified`, `msgpack` |
| `put` | `key`, `type`, `value`, optional `ttl`, optional `mode` | `key`, `type` |
| `add` | `prefix` (`place:`), `value`, optional `type` (needed unless the value is an array or an object), optional `ttl` | `key`, `id` (the key after the prefix), `type` ([6.7](#67-add--let-kivadb-name-the-key)) |
| `del` | `key` | `deleted` |
| `stats` | - | The figures of the `stats` command, as fields |
| `users` | - | `users`: name, role, grants, default workspace, created, whether the password is empty (admin only) |
| `query` | `target` (`user:*.age`), `cmp` (`=` `!=` `>` `>=` `<` `<=` `in` `like` `ilike`), `value`, `limit`; or `target` (`user:*`) and `where` (below); `order_by` (`"created_at desc"`), `after` (a key), `reverse` | `rows`: key and document; `plan` ([11.6](#116-what-a-query-does-not-do)) |
| `aggregate` | `target` (`user:*`), optional `where`, `group_by`, `compute`, `limit` | `matches` and `values`; with `group_by`, `groups`: `key`, `matches`, `values`; `plan` |
| `explain` | the same as `query`; `analyze`: `true` to run it | `plan`: strategy, index, candidates, total keys, order; `suggestion`: the index to create, if any; with `analyze`, `analyze`: the counts and times of [11.5](#115-explain--see-how-a-query-will-run) |
| `similar` | `target` (`doc:*.embedding`), `vector`, `limit`, `metric` | `results`: key, score, document; `compared`; `indexed` |
| `changes` | `cursor` (`beginning`, `now` or a cursor), `consumer`, `limit` (100), `pattern`, `wait` (milliseconds, at most 30 000) | As `GET /v1/changes` ([22.5](#225-the-change-journal-changes-kept-read-again-from-a-cursor)): `cursor`, `more`, `changes`... |

Notes on `query` and `aggregate`:

* `where` is the condition of [11.1](#111-the-condition) as JSON: `{"field": "age", "cmp": ">", "value": 18}` (`cmp` defaults to `=`), `{"role": "dev", "city": "Goma"}` (equalities joined by `and`; a list as value means `in`: `{"city": ["Goma", "Bukavu"]}`), `{"and": [...]}`, `{"or": [...]}`, `{"not": {...}}`, or a list (its items joined by `and`). With `where`, `target` is the key pattern alone.
* Dates: a value `{"$datetime": "2026-10-27T13:30:00Z"}` compares the field as a moment, `{"$time": "08:00"}` as a time of day; a field written `"month(created_at)"` compares a part of its date ([11.1](#111-the-condition)), and so does `"group_by": "month(created_at)"`.
* `compute` names each aggregate: `{"n": "count", "mean_age": {"avg": "age"}, "top": {"max": "salary"}}`, or a list `["count", {"avg": "age"}]` whose results are named `count`, `avg_age`. Without it, `count`. At most 32 per request.

```json
{"op": "aggregate", "target": "user:*", "where": {"not": {"role": "ops"}}, "group_by": "role",
 "compute": {"n": "count", "mean_age": {"avg": "age"}}}
```

answers `{"matches": 3, "groups": [{"key": "admin", "matches": 1, "values": {"n": 1, "mean_age": 30}}, {"key": "dev", "matches": 2, "values": {"n": 2, "mean_age": 33}}], "plan": {...}}`.

Notes on `put`:

* `value` is plain JSON matching `type`. A number may be given as a JSON number or as text (`"3.14"`), bytes are base64 text. A `datetime` is a text (`"2026-10-27T13:30:00Z"`, or a wall clock of the session's zone such as `"2026-10-27 14:30"`) or milliseconds since 1970; a `date` is `"2026-10-27"`, a `time` `"14:30:00"`. `get` answers a datetime in ISO 8601, in UTC.
* `mode` is `create` (fail if the key exists), `update` (fail if it does not) or `upsert` (default).
* A write that a unique index refuses ([11.7](#117-unique-indexes)) fails with a message starting with `UNIQUE:`; over HTTP, with code 409.
* `ttl` absent keeps the current expiry of the key, `0` removes it, a positive number sets it in seconds.

Because values travel as JSON, they are free of the shell's quoting rules: a text may contain both kinds of quotes.

Request and answer for reading a key:

```json
{"op": "get", "key": "user:1"}
```

```json
{"found": true, "key": "user:1", "type": "object", "value": {"name": "Alice"}, "ttl": null, "modified": 1791791094, "msgpack": false}
```

An error comes back with status 1 and the message as payload, as for every request. The Rust client exposes this as `Client::api`. Over HTTP, send the same JSON to `POST /v1/api`.

---

## Chapter 21 — The REST API

The REST API lets anything that speaks HTTP use KivaDB: a browser, `curl`, any programming language. It listens on port **8080**.

### 21.1 The shape of answers

Every answer is JSON.

```json
{"status": "ok", "data": "...", "timestamp": 1791791113}
```

```json
{"status": "error", "error": "key 'ghost' not found", "timestamp": 1791791113}
```

| HTTP code | Meaning |
| :--- | :--- |
| 200 | Success |
| 400 | The request is malformed or the value is invalid |
| 401 | Credentials are missing or wrong |
| 403 | You are not allowed to do this |
| 404 | The key or the route does not exist |
| 409 | Conflict (for example: the node is a read-only replica) |
| 429 | Too many requests |

### 21.2 Keys

**Write** - `POST /v1/key/{key}` creates the key or replaces it. The body is the value, in JSON:

```bash
curl -X POST http://localhost:8080/v1/key/user:1 -d '{"name": "Alice", "role": "admin"}'
```

```json
{"key":"user:1","type":"object","status":"ok","timestamp":1791791113}
```

To set an expiry, wrap the value: `{"value": ..., "ttl": seconds}` (or add `?ttl=60` to the URL).

```bash
curl -X POST http://localhost:8080/v1/key/session:42 -d '{"value": "abc123", "ttl": 3600}'
```

The JSON type decides the KivaDB type: a JSON string becomes a string, a number a number, and so on. A body that is not valid JSON is stored as a string.

**Read** - `GET /v1/key/{key}`:

```bash
curl http://localhost:8080/v1/key/user:1
```

```json
{"key":"user:1","type":"object","data":{"name":"Alice","role":"admin"},"ttl":null,"status":"ok","timestamp":1791791113}
```

`ttl` is the number of seconds left, or `null`. Add `?path=.name` to read one field of a document. Values of type bytes are returned in base64.

**Delete** - `DELETE /v1/key/{key}`:

```bash
curl -X DELETE http://localhost:8080/v1/key/user:1
```

```json
{"key":"user:1","deleted":true,"status":"ok","timestamp":1791791113}
```

### 21.3 Listings

| Route | Parameters | Returns |
| :--- | :--- | :--- |
| `GET /v1/keys` | `from`, `to`, `limit`, `reverse=true` | Key names, in order |
| `GET /v1/scan` | `pattern`, `limit` | Keys with type, size, modification time, ttl |
| `GET /v1/range` | `from`, `to`, `limit`, `reverse=true` | Keys with their values |

```bash
curl "http://localhost:8080/v1/keys?from=user:&to=user:9&limit=2"
```

```json
{"count":2,"data":["user:1","user:2"],"status":"ok","timestamp":1791791113}
```

### 21.4 Any command

`POST /v1/exec` runs a shell command and returns what it printed. Everything in this manual is therefore available over HTTP.

```bash
curl -X POST http://localhost:8080/v1/exec -d '{"command": "count where user:*.age > 26"}'
```

```json
{"ok":true,"output":["Count: 2"],"status":"ok","timestamp":1791791113}
```

`ok` is `false` when the command reported an error. Each HTTP request is independent: a transaction cannot span several requests.

### 21.5 Other routes

| Route | Purpose |
| :--- | :--- |
| `GET /v1/stats` | Engine statistics as JSON |
| `GET /metrics` | Prometheus metrics ([Chapter 27](#chapter-27--monitoring)) |
| `GET /ws` | WebSocket change notifications ([Chapter 22](#chapter-22--change-notifications)) |
| `GET /v1/stream` | Server-Sent Events change notifications ([22.4](#224-over-server-sent-events)) |
| `POST /v1/api` | Structured requests, including `query` and `similar` ([20.4](#204-structured-requests-for-tools)) |
| `GET /health` | `{"version":"1.7.1","status":"ok","timestamp":...}` - no login needed |
| `GET /docs` | An interactive page listing the routes, with a "Try it" button for each |
| `GET /openapi.json` | The same description in OpenAPI format |

### 21.6 Authentication and workspaces

With `auth_required`, send the user name and password with HTTP Basic authentication:

```bash
curl -u fordi:mon_pass http://localhost:8080/v1/key/user:1
```

Without valid credentials the answer is `401`. Requests are executed in the workspace the user lands in ([16.5](#165-logging-in)); add `?workspace=<name>` to work in another one you have access to, or log in as `<user>@<workspace>`:

```bash
curl -u fordi:mon_pass "http://localhost:8080/v1/key/product:1?workspace=shop"
curl -u fordi@travel.places:mon_pass http://localhost:8080/v1/key/place:1
```

A user who lands in no workspace and names none gets `403` with `you are in no workspace` on requests that read or write data.

### 21.7 CORS and rate limiting

* **CORS.** By default the API sends the headers that allow a web page served from another site to call it (`Access-Control-Allow-Origin: *`). Turn it off with `config set cors false`.
* **Rate limiting.** Each client address may send `http.rate_limit` requests per minute (600 by default). Beyond that the answer is `429` with a `Retry-After` header. Set it to `0` to disable.

The REST API speaks plain HTTP. For HTTPS, put a reverse proxy in front of it.

---

## Chapter 22 — Change notifications

Instead of asking again and again whether something changed, a client can **subscribe** and be told.

### 22.1 In the shell

```text
kiva> subscribe user:*
Subscribed to 'user:*' in workspace 'default'. Changes will be announced as they happen.
```

When anyone - another shell, a program, an HTTP request - creates, changes or deletes a matching key, a line appears:

```text
[event] set user:7 (object)
[event] update user:7 (object)
[event] del user:7
```

| Event | Meaning |
| :--- | :--- |
| `set <key> (<type>)` | The key was created |
| `update <key> (<type>)` | An existing key was overwritten |
| `del <key>` | The key was deleted |
| `reset (all keys deleted)` | `del all keys` was run |

`unsubscribe` stops everything; `unsubscribe user:*` stops one pattern. Subscriptions belong to the workspace you were in when you subscribed and end when you leave it or disconnect.

### 22.2 Over TCP

On a text connection, notifications arrive as lines starting with `[event] `. On a binary connection they arrive as frames with status 9, at any moment, including between a request and its reply; a client must set them aside and keep waiting for its reply.

### 22.3 Over WebSocket

Web pages and applications can use the WebSocket endpoint of the REST port:

```javascript
const ws = new WebSocket("ws://localhost:8080/ws?pattern=user:*");
ws.onmessage = (message) => console.log(JSON.parse(message.data));
```

The first message confirms the subscription; then one message per change:

```json
{"event":"subscribed","pattern":"user:*","workspace":"default"}
{"event":"set","key":"user:5","type":"object","timestamp":1791791113}
{"event":"del","key":"user:5","type":null,"timestamp":1791791114}
```

Without `pattern`, every key is watched. With authentication on, the WebSocket request must carry the same Basic credentials as any other request.

### 22.4 Over Server-Sent Events

Server-Sent Events (SSE) is plain HTTP that stays open and receives messages, one way. It is simpler than a WebSocket, passes through proxies easily, and browsers support it natively with `EventSource`.

```bash
curl -N "http://localhost:8080/v1/stream?pattern=order:*"
```

```text
retry: 2000

event: subscribed
data: {"event":"subscribed","pattern":"order:*","workspace":"default"}

id: 1
event: set
data: {"event":"set","key":"order:7","type":"object","timestamp":1791791113}
```

In a web page:

```javascript
const stream = new EventSource("http://localhost:8080/v1/stream?pattern=order:*");
stream.addEventListener("set", (e) => console.log(JSON.parse(e.data)));
stream.addEventListener("del", (e) => console.log(JSON.parse(e.data)));
```

A comment line is sent every 15 seconds so that idle connections stay open. With authentication on, send Basic credentials (from a browser, a reverse proxy usually adds them). `config set stream false` turns the endpoint off.

These notifications are not kept: a client that was not connected misses them. To read the changes again later, from where a program stopped, use the change journal (22.5).

### 22.5 The change journal: changes kept, read again from a cursor

The notifications above go to whoever listens at that moment: a program that was disconnected, or not started yet, misses them. The **change journal** keeps every committed write of a workspace on disk, numbered, for a retention period, so that a program (a search index, a cache, an analytics pipeline, another database) can read them in order and resume where it stopped, after a disconnection or a restart of either side.

```text
kiva> changes enable
Change journal started for workspace 'default': keys and values, kept 1 day(s), at most 256.00 MB.
kiva> set user:1 {"name": "Ana"}
kiva> update user:1 {"name": "Ana", "age": 31}
kiva> begin
kiva> set order:1 {"total": 40} and del cart:ana
kiva> commit
kiva> changes read
#1 2026-10-27T14:30:00.000Z insert user:1 (object) {"name":"Ana"}
#2 2026-10-27T14:30:05.000Z update user:1 (object) {"name":"Ana","age":31}
#3 2026-10-27T14:30:09.000Z insert order:1 (object) {"total":40} [transaction 3]
#4 2026-10-27T14:30:09.000Z delete cart:ana [transaction 3, end]
Cursor: 54a087b71d59813c0000000000000004
kiva> changes read from 54a087b71d59813c0000000000000004
No change after this point.
Cursor: 54a087b71d59813c0000000000000004
```

The journal starts empty when it is enabled: the writes made before are not in it.

| Command | Effect |
| :--- | :--- |
| `changes enable [keys only] [retain <24h>] [max <256mb>]` | Start the journal of the current workspace, or change its settings (admin). `keys only` keeps the keys and the operations, not the values |
| `changes read [from beginning\|now\|<cursor>] [as <consumer>] [limit N] [like <pattern>]` | The changes after a point (20 by default, at most 1 000), and the cursor to go on from |
| `changes status` | Settings, changes kept, size, named consumers and how far behind each one is |
| `changes forget <consumer>` | Forget a named consumer (admin) |
| `changes disable` | Stop and remove the journal; its cursors are no longer valid (admin) |

**From programs**, `GET /v1/changes` on the REST port answers JSON:

```bash
curl "http://localhost:8080/v1/changes?from=beginning&limit=100"
curl "http://localhost:8080/v1/changes?cursor=54a087b71d59813c0000000000000004&wait=25000"
```

```json
{"workspace": "default", "cursor": "54a087b71d59813c0000000000000004", "more": false,
 "first_seq": 1, "last_seq": 4, "changes": [
  {"seq": 3, "cursor": "54a087b71d59813c0000000000000003", "op": "set", "created": true,
   "key": "order:1", "type": "object", "value": {"total": 40},
   "timestamp": "2026-10-27T14:30:09.000Z", "expires_at": null, "txn": 3, "txn_end": false},
  {"seq": 4, "cursor": "54a087b71d59813c0000000000000004", "op": "delete", "created": null,
   "key": "cart:ana", "type": null, "value": null,
   "timestamp": "2026-10-27T14:30:09.000Z", "expires_at": null, "txn": 3, "txn_end": true}]}
```

| Parameter | Meaning |
| :--- | :--- |
| `cursor` (or `from`) | Read after this cursor; `beginning` (the oldest change kept) or `now` (only those to come) |
| `consumer` | A name under which KivaDB remembers the position (below) |
| `limit` | Changes per answer: 100 by default, at most 1 000 |
| `pattern` | Only the keys matching it; the cursor still moves past the others |
| `wait` | Milliseconds to wait for a change when there is none yet (at most 30 000): a long poll, answered as soon as one comes |

`op` is `set` (with `created`: `true` for a new key, `false` for a key overwritten), `delete` or `reset` (`del all keys`). `value` is the value written, as the REST API gives it (bytes in base64, dates as ISO 8601 in UTC), `null` with `keys only`. The changes of one transaction carry the number of its first change in `txn`, and its last one has `txn_end`: a program can apply a transaction whole. Any other command that changes keys gives its changes too: `json set` and the other field changes, Redis clients, scripts, the REST API, the defaults of a schema ([11.8](#118-schemas-rules-for-the-documents-of-a-pattern)).

#### What is guaranteed

* **Only what was committed, in commit order.** A rolled-back transaction, or one a crash cut before its commit record, gives no change. A change is handed to a reader only once its data and itself are on disk: a read syncs both first, whatever `storage.sync_writes` says.
* **Data and changes agree after a crash.** The data file stays the source of truth. Each change records where its data ends; when KivaDB opens a workspace, the changes whose data the crash lost are dropped (no reader ever got them), and those the journal lacks are rebuilt from the data file, with the same numbers. A change rebuilt that way says `"created": null`: whether the key was new is not known any more.
* **Cursors** are opaque texts (the identity of the journal and a number). A cursor stands for a change: reading from it gives the changes **after** it, and the answer's `cursor` is where to go on. Cursors survive restarts and compactions (which remove old values from the data file, not changes from the journal).
* **At least once.** A reader may get a change again: after a restart that rebuilt it, or when it reads from a cursor older than what it had handled. Make applying a change idempotent (`seq` is unique in a journal) rather than counting on exactly once.
* **Expired and invalid cursors are refused**, never silently skipped. Past the retention, `CURSOR_EXPIRED: ...` (the oldest number kept is in the message): resynchronise, then read from `now` or the beginning. `CURSOR_INVALID: ...` for a cursor of another journal: another node, a journal disabled and enabled again, or restored from a backup.

#### Retention, slow readers, consumers and at-least-once delivery

The journal keeps changes for `retain` (one day by default) and at most `max` on disk (256 MB by default), in segment files that are removed whole, oldest first. Readers never hold writes back: a reader that is too slow, or stopped, finds its cursor expired rather than the disk filled.

With `as <consumer>` (`consumer=` over REST), KivaDB remembers a position per name. The position moves when the consumer reads **from a cursor**: that says it handled everything up to it. Reading as the consumer without a cursor resumes after its last acknowledgement, so a batch it got but never acknowledged (it crashed while applying it) comes again. `changes status`, and `kiva_changes_consumer_lag{consumer=...}` in Prometheus, show how far behind each one is. A reader without a name keeps its cursor itself.

#### Security

Reading the journal needs read access to the workspace, checked at every read (and at every look of a `wait`): a grant revoked meanwhile ends the next read with `403`. Enabling, disabling and forgetting consumers need the admin role. With `keys only`, values (personal data, secrets) never reach the journal. In an encrypted workspace ([16.12](#1612-encryption-at-rest)) the journal is encrypted with the same key; a key rotation re-encrypts the data file, while the journal's older changes stay readable as long as the old key is given, and leave with the retention.

#### Limits

* **Per node.** Each node keeps its own journal and numbers its own changes. A replica with `changes enable` journals what its master sends; a cursor of the master means nothing there (`CURSOR_INVALID`). After a failover, readers resynchronise from the new master.
* **Expirations** (keys whose TTL ran out) are not changes: they are not written to the data file either. A reader that mirrors keys with a TTL applies the `expires_at` of their changes.
* **Backups** hold the data, not the journal: a restored workspace starts a new journal (its cursors are new), as does a reset cut short by a crash.
* Writes cost a little more with a journal (the record is written twice, the value once more unless `keys only`); a workspace without one pays nothing.
* KivaDB 1.x ignores the journal (`store.kiva.changes`), which does not change the data file.

---

## Chapter 23 — Redis compatibility

### 23.1 What it gives you

Redis is the most used key-value server; thousands of applications and libraries speak its protocol (RESP). KivaDB speaks it too, on port **6379**, so an application written for Redis can use KivaDB **by changing only its connection address**. Each version is checked against the current releases of four widely used Node.js libraries, run unchanged: **ioredis**, **node-redis** (which uses RESP3), **rate-limiter-flexible** and **rate-limit-redis** (the store of express-rate-limit). The checks are in `tests/redis-clients`.

```bash
redis-cli -p 6379
```

```text
127.0.0.1:6379> SET greeting "hello"
OK
127.0.0.1:6379> INCR visits
(integer) 1
127.0.0.1:6379> HSET user:1 name Alice city Kinshasa
(integer) 2
```

The data is the same as everywhere else in KivaDB:

```text
kiva> get user:1.name and visits
user:1.name: Alice [string]
visits: 1 [number]
```

Unlike a default Redis, every write is in the data file before the answer: keys, counters and their expiry times survive a crash or a restart of the server. With `sync_writes = true`, they also survive a power failure, at a cost kept low by group commit ([17.2](#172-what-survives-what)). A failing Lua script leaves nothing behind ([23.5](#235-lua-scripts-eval)), and the same data is reachable as documents and vectors ([23.11](#2311-json-documents-redisjson-commands)).

### 23.2 How types match

| Redis | KivaDB |
| :--- | :--- |
| string | `string` when the bytes are UTF-8 text, `bytes` otherwise. `INCR` and friends store a `number` |
| hash | `object` whose fields hold strings |
| list | `array` of strings |
| set | `object` `{"redis": "set", "members": ["a", "b"]}` ([23.8](#238-sets-and-sorted-sets)) |
| sorted set | `object` `{"redis": "zset", "members": {"alice": 12.5}}` |

KivaDB values read through Redis: strings, numbers, booleans, null and bytes come back as Redis strings, and so do dates, times and datetimes, in ISO 8601 (a datetime in UTC: `2026-10-27T13:30:00.000Z`); objects are hashes and arrays are lists (`GET` on them answers `WRONGTYPE`, as Redis does). An object marked `"redis": "set"` or `"zset"` is a set or a sorted set, never a hash.

### 23.3 Supported commands

| Group | Commands |
| :--- | :--- |
| Connection | `PING`, `ECHO`, `AUTH`, `HELLO` (protocol 2 or 3), `SELECT` (0 to 15, see [23.4](#234-logins-databases-roles)), `CLIENT`, `QUIT`, `COMMAND` (`COUNT`, `LIST`), `INFO`, `TIME` |
| Strings | `GET`, `SET` (with `EX`, `PX`, `EXAT`, `PXAT`, `NX`, `XX`, `KEEPTTL`, `GET`), `SETNX`, `SETEX`, `PSETEX`, `GETSET`, `GETDEL`, `GETEX`, `MGET`, `MSET`, `MSETNX`, `APPEND`, `STRLEN`, `GETRANGE` (`SUBSTR`), `SETRANGE`, `INCR`, `DECR`, `INCRBY`, `DECRBY`, `INCRBYFLOAT` |
| Keys | `DEL`, `UNLINK`, `EXISTS`, `TYPE`, `TOUCH`, `KEYS`, `SCAN`, `RENAME`, `RENAMENX`, `COPY` (`REPLACE`), `EXPIRE`, `PEXPIRE`, `EXPIREAT`, `PEXPIREAT` (all with `NX`, `XX`, `GT`, `LT`), `TTL`, `PTTL`, `EXPIRETIME`, `PEXPIRETIME`, `PERSIST`, `DBSIZE`, `FLUSHDB`, `FLUSHALL` |
| Hashes | `HSET`, `HMSET`, `HSETNX`, `HGET`, `HMGET`, `HGETALL`, `HKEYS`, `HVALS`, `HLEN`, `HEXISTS`, `HSTRLEN`, `HDEL`, `HINCRBY`, `HINCRBYFLOAT`, `HSCAN` (`MATCH`, `NOVALUES`), `HRANDFIELD` (`WITHVALUES`) |
| Lists | `LPUSH`, `RPUSH`, `LPUSHX`, `RPUSHX`, `LPOP`, `RPOP`, `LMPOP`, `LLEN`, `LRANGE`, `LINDEX`, `LPOS` (`RANK`, `COUNT`, `MAXLEN`), `LSET`, `LINSERT`, `LREM`, `LTRIM`, `LMOVE`, `RPOPLPUSH` |
| Blocking | `BLPOP`, `BRPOP`, `BRPOPLPUSH`, `BLMOVE`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX`, `BZMPOP`, `WAIT` ([23.12](#2312-blocking-commands-and-watch)) |
| Streams | `XADD` (`NOMKSTREAM`, `MAXLEN`, `MINID`), `XLEN`, `XRANGE`, `XREVRANGE`, `XDEL`, `XTRIM`, `XSETID`, `XREAD` (`BLOCK`), `XGROUP`, `XREADGROUP` (`BLOCK`, `NOACK`), `XACK`, `XPENDING`, `XCLAIM`, `XAUTOCLAIM`, `XINFO` ([23.13](#2313-streams)) |
| Sets | `SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SMISMEMBER`, `SCARD`, `SPOP`, `SRANDMEMBER`, `SMOVE`, `SINTER`, `SUNION`, `SDIFF`, `SINTERSTORE`, `SUNIONSTORE`, `SDIFFSTORE`, `SINTERCARD`, `SSCAN` ([23.8](#238-sets-and-sorted-sets)) |
| Sorted sets | `ZADD` (`NX`, `XX`, `GT`, `LT`, `CH`, `INCR`), `ZINCRBY`, `ZREM`, `ZSCORE`, `ZMSCORE`, `ZCARD`, `ZCOUNT`, `ZLEXCOUNT`, `ZRANK`, `ZREVRANK` (`WITHSCORE`), `ZRANGE` (`BYSCORE`, `BYLEX`, `REV`, `LIMIT`, `WITHSCORES`), `ZRANGESTORE`, `ZREVRANGE`, `ZRANGEBYSCORE`, `ZREVRANGEBYSCORE`, `ZRANGEBYLEX`, `ZREVRANGEBYLEX`, `ZPOPMIN`, `ZPOPMAX`, `ZREMRANGEBYRANK`, `ZREMRANGEBYSCORE`, `ZREMRANGEBYLEX`, `ZRANDMEMBER`, `ZSCAN`, `ZUNION`, `ZINTER`, `ZDIFF`, `ZUNIONSTORE`, `ZINTERSTORE`, `ZDIFFSTORE`, `ZINTERCARD` |
| Transactions | `MULTI`, `EXEC`, `DISCARD`, `WATCH`, `UNWATCH` ([23.12](#2312-blocking-commands-and-watch)) |
| Scripts | `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`, `SCRIPT LOAD`, `SCRIPT EXISTS`, `SCRIPT FLUSH` ([23.5](#235-lua-scripts-eval)) |
| Publish/subscribe | `PUBLISH`, `SUBSCRIBE`, `UNSUBSCRIBE`, `PSUBSCRIBE`, `PUNSUBSCRIBE`, `PUBSUB CHANNELS`, `PUBSUB NUMSUB`, `PUBSUB NUMPAT` ([23.7](#237-publish-and-subscribe)) |
| JSON | `JSON.SET` (`NX`, `XX`), `JSON.GET`, `JSON.MGET`, `JSON.DEL`, `JSON.FORGET`, `JSON.TYPE`, `JSON.NUMINCRBY`, `JSON.NUMMULTBY`, `JSON.STRLEN`, `JSON.ARRLEN`, `JSON.OBJLEN`, `JSON.OBJKEYS`, `JSON.ARRAPPEND`, `JSON.ARRINSERT` ([23.11](#2311-json-documents-redisjson-commands)) |
| KivaDB | `KIVA.SIMILAR` (vector search), `KIVA.API` (any structured request) ([23.10](#2310-vector-search-and-the-semantic-cache-from-a-redis-client)) |

**Lists as queues.** `LMOVE` takes a job from a queue and puts it in a processing list in one step, so a worker that crashes never loses it; `LTRIM` keeps a list to its last entries:

```text
127.0.0.1:6379> LMOVE q:jobs q:processing LEFT RIGHT
"j1"
127.0.0.1:6379> LREM q:processing 1 j1
(integer) 1
127.0.0.1:6379> LPUSH log:recent e9
(integer) 6
127.0.0.1:6379> LTRIM log:recent 0 4
OK
```

`COPY` works within one database: `COPY ... DB n` is refused, because each database is a separate workspace. `TOUCH` only counts the keys that exist, since KivaDB keeps no access time.

Pipelining works, in RESP2 and RESP3. `COMMAND LIST` answers exactly the commands above, `COMMAND COUNT` how many there are.

**Commands KivaDB refuses.** Any other command answers the error Redis gives for an unknown command, so client libraries recognize it and an application finds out at once:

```text
127.0.0.1:6379> GEOADD shops 15.3 -4.3 gombe
(error) ERR unknown command 'GEOADD', with args beginning with: 'shops' '15.3' '-4.3' 'gombe'
```

For the families left out on purpose, the same message adds why, and what to use instead:

| Family | Commands | Instead |
| :--- | :--- | :--- |
| Cluster | `CLUSTER`, `READONLY`, `READWRITE`, `ASKING`, `SPUBLISH`, `SSUBSCRIBE` | Connect as to a standalone Redis: one server holds a workspace's data |
| Functions | `FUNCTION`, `FCALL`, `FCALL_RO` | `EVAL`, `EVALSHA` ([23.5](#235-lua-scripts-eval)) |
| Replication | `REPLICAOF`, `SLAVEOF`, `SYNC`, `PSYNC`, `FAILOVER` | KivaDB's own replication: `replica start` ([Chapter 25](#chapter-25--replication)) |
| Modules | `MODULE` | JSON documents and `KIVA.SIMILAR` are built in |

```text
127.0.0.1:6379> FCALL refill 1 stock
(error) ERR unknown command 'FCALL', with args beginning with: 'refill' '1' 'stock' (KivaDB does not support Redis functions: send the Lua code with EVAL or EVALSHA.)
```

A refused command inside `MULTI` is not queued, and the whole transaction is then discarded: `EXEC` answers `EXECABORT`, as in Redis, and nothing of it is applied. Inside a script, `redis.call` raises the same error.

**One command at a time.** Like Redis, KivaDB runs the Redis commands of all connections one after the other. `INCR` from many clients at once never loses a step, and a `MULTI ... EXEC` block or a script is never interleaved with another client's commands.

**Expiry precision.** KivaDB keeps expiry times to the millisecond. A key set with `PX 150`, `PEXPIRE`, `PSETEX` or `PXAT` is gone 150 ms later, not at the next second; `PTTL` and `PEXPIRETIME` answer milliseconds, and `TTL` rounds to the nearest second as Redis does. A key read after its deadline is never returned, and expired keys leave memory within a second ([7.3](#73-what-happens-at-expiry)).

### 23.4 Logins, databases, roles

Everything KivaDB enforces applies here too:

* With authentication on, the client must log in. `AUTH <password>` logs in as `kiva` (Redis's "default" user); `AUTH <user> <password>` as any account. In a URL: `redis://kiva:password@host:6379`.
* **A workspace by its name.** Log in as `<user>@<workspace>` to land in that workspace, or send `SELECT <workspace>`:

  ```js
  // ioredis: the @ of the user name is written %40 in a URL
  const places = new Redis("redis://app%40travel.places:secret@localhost:6379");
  const cache = new Redis("redis://app%40travel.cache:secret@localhost:6379");
  ```

  ```text
  127.0.0.1:6379> SELECT travel.places
  OK
  ```

  Clients send the login again at every reconnection, so a connection opened as `app@travel.places` stays in `travel.places` even after the network drops. A `SELECT` by name is sent again by node-redis (`createClient({ ..., database: "travel.places" })`), but not by ioredis or `redis-cli`, which only keep numbered databases: with ioredis, put the workspace in the user name. A workspace the user cannot reach is refused with `NOPERM`, and the error says which grant is missing.
* **Database 0** is the workspace the user lands in: their default one (`modify user "app" default workspace travel.places`), `default` for an administrator, or the only workspace they can reach. A user who can reach several and has no default lands in none: every command on data then answers `you are in no workspace` until a `SELECT`.
* **Numbered databases are workspaces too.** `SELECT 2` (or `/2` at the end of a URL) uses the workspace `db2`, and so on up to 15. An administrator's first `SELECT n` creates `db<n>`; another user needs it to exist and to be granted access, and the error message says how:

  ```text
  workspace create db2
  grant write on workspace "db2" to "pay"
  ```

  Two applications can therefore keep the separation they had in Redis (one on database 0, one on database 2). Names say more than numbers: a project per application, with a workspace per use (`travel.places`, `travel.cache`), is clearer ([14.6](#146-projects)).
* A viewer cannot write (`NOPERM`), a replica refuses writes (`READONLY`), `FLUSHDB` needs the admin role. `FLUSHDB` and `FLUSHALL` empty the current workspace only.
* `MULTI ... EXEC` runs as a KivaDB transaction: all or nothing, and `EXEC` answers nil if another session (through another protocol) changed the same keys meanwhile.

### 23.5 Lua scripts (EVAL)

Libraries such as rate limiters send small Lua scripts that read and write several keys in one step. KivaDB runs them with the same `redis` API as Redis:

```text
127.0.0.1:6379> EVAL "local n = redis.call('INCR', KEYS[1]) if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end return n" 1 rl:login:1.2.3.4 900
(integer) 1
```

* `KEYS` and `ARGV` hold the keys and arguments; `redis.call` runs a command and raises its error, `redis.pcall` returns the error as a table; `redis.error_reply`, `redis.status_reply`, `redis.sha1hex` and `redis.log` exist.
* Replies convert as in Redis: integers to numbers, strings to strings, a missing value to `false`, arrays to tables; in the other direction, numbers are truncated to integers and a table becomes an array.
* `EVALSHA` runs a script by its SHA-1 after `SCRIPT LOAD` or a first `EVAL`; an unknown one answers `NOSCRIPT`, which clients handle by sending `EVAL`. The cache lives until KivaDB stops or `SCRIPT FLUSH`.
* Scripts run in the sandbox of [26.5](#265-the-sandbox) (no files, no operating system, the same time and memory limits) and cannot run `MULTI`, `EVAL`, `SELECT`, `AUTH` or `SUBSCRIBE`. They may `PUBLISH`: the messages leave when the script succeeds.
* **All or nothing.** A script's writes are applied together when it ends. If it fails, none of them is kept. Redis keeps the writes made before the error: KivaDB is stricter here, never looser.
* Each command inside the script is checked against the user's role, as if sent directly.

### 23.6 Replacing Redis in an application

KivaDB fits the uses where Redis serves as short-lived memory: pending sign-ups with a code that expires, "resend" locks, login and 2FA attempt counters, sessions, caches. The steps on a server where Redis already runs:

1. **Install KivaDB and make it a service** ([29.3](#293-linux-systemd)), so that it starts with the machine. An application that depends on it for sign-ups must never find it stopped.
2. **Secure it**: a password for `kiva` and authentication on ([16.6](#166-turning-authentication-on)). Create one account per application; give each the database it used (`grant`), or, better, a project with a workspace per use ([14.6](#146-projects)) and a default workspace as database 0.
3. **Run it next to Redis first**: `kivadb -e "config set resp.port 6380"`, restart KivaDB, and point one application at `redis://<user>:<password>@127.0.0.1:6380` (add `/2` if it used database 2). Nothing else changes in its code.
4. **Check** the application's flows: sign-up and code, resend, wrong passwords until the limit.
5. **Switch the rest**, then stop Redis (`systemctl disable --now redis-server`) and, if you like, give port 6379 back to KivaDB.

Data that lives a few minutes does not need to be copied: at worst, a sign-up in progress at the moment of the switch asks for a new code. Before switching, check with `MONITOR` on Redis, or by reading the code, that the application only uses commands of [23.3](#233-supported-commands).

### 23.7 Publish and subscribe

An application that runs as several instances can tell all of them that something changed, for example that its texts were edited and must be reloaded. One connection listens, the others publish:

```text
# connection 1
127.0.0.1:6379> SUBSCRIBE app:texts:changed
1) "subscribe"
2) "app:texts:changed"
3) (integer) 1
1) "message"
2) "app:texts:changed"
3) "fr"

# connection 2
127.0.0.1:6379> PUBLISH app:texts:changed fr
(integer) 1
```

`PUBLISH` answers how many listeners received the message. `PSUBSCRIBE app:texts:*` listens to every channel matching a pattern (`*`, `?`, `[abc]`, `[a-z]`, `[^a]` and `\` to escape, as in Redis). `PUBSUB CHANNELS`, `PUBSUB NUMSUB` and `PUBSUB NUMPAT` show who listens. Client libraries work unchanged: ioredis (a connection used for `subscribe`), node-redis (`client.duplicate()` then `subscribe`); both are checked with every change of KivaDB.

* **Messages are not stored.** As in Redis, a message reaches the connections listening at that moment; one that connects later never sees it, and nothing is written to disk. Keep the state itself in keys, and use messages to say "go and read it again".
* **Channels belong to the workspace of database 0**, the one the connection landed in. Applications in different workspaces never hear each other's channels; a user who landed in no workspace has channels of their own. `SELECT` does not change them, as in Redis: a publisher on `/2` reaches a listener on `/0` of the same user.
* **A listening connection** in RESP2 accepts only `SUBSCRIBE`, `PSUBSCRIBE`, `UNSUBSCRIBE`, `PUNSUBSCRIBE`, `PING` and `QUIT`, like Redis; libraries use a separate connection for it. In RESP3 (`HELLO 3`), messages arrive as push frames between replies and every command stays allowed.
* **Inside `MULTI` or a script**, published messages leave when it commits, and never when it fails.
* **A listener that stops reading** is disconnected once 65,536 messages wait for it, so it cannot slow the publishers or fill the server's memory (Redis does the same with its output buffer limit).
* **One server.** Messages reach the clients of the KivaDB they were published on; replicas do not forward them.
* These are Redis channels. To be told when *keys* change, use `subscribe` on port 2911, WebSocket or Server-Sent Events ([Chapter 22](#chapter-22--change-notifications)).

### 23.8 Sets and sorted sets

A **set** holds distinct members: tags of a post, users who already voted, tokens already used. A **sorted set** gives each member a score and keeps them ordered by it: leaderboards, priority queues, and the sliding-window rate limits many libraries build with `ZADD`, `ZREMRANGEBYSCORE` and `ZCARD`.

```text
127.0.0.1:6379> ZADD game:lb 120 amani 80 baraka 200 chiku
(integer) 3
127.0.0.1:6379> ZINCRBY game:lb 100 baraka
"180"
127.0.0.1:6379> ZREVRANGE game:lb 0 1 WITHSCORES
1) "chiku"
2) "200"
3) "baraka"
4) "180"
```

They are stored as KivaDB objects, so they are written to disk, replicated and encrypted like everything else, and readable from the shell:

```text
kiva> get game:lb
game:lb: {"redis":"zset","members":{"amani":120,"baraka":180,"chiku":200}} [object]
```

* Scores are 64-bit floating-point numbers; `+inf` and `-inf` are accepted (saved as the text `"inf"` and `"-inf"`, which JSON has no number for). Members of equal score are ordered by their bytes, as in Redis.
* `TYPE` answers `set` or `zset`; `SCAN ... TYPE set` finds them. Hash, list and string commands on them answer `WRONGTYPE`, and the other way round.
* An emptied set or sorted set disappears, as in Redis. Changing one keeps its expiry; `...STORE` commands replace the destination and its expiry.
* In RESP3 (`HELLO 3`, node-redis), scores come back as doubles, `SMEMBERS` as a set and `WITHSCORES` as pairs; in RESP2, as text and flat lists. Both were checked with ioredis and node-redis.
* `SSCAN` and `ZSCAN` return every member in one answer (cursor `0`); `MATCH` follows Redis patterns.
* **Size.** Each command reads and rewrites the whole value, and a sorted set is ordered when it is read: right for up to a few thousand members per key (a rate-limit window, the tags of a post, the leaderboard of a game or a shop). For millions of members in one key, a real Redis remains the better tool.
* **A size limit.** A set, a sorted set or a list may not grow past `resp.max_collection_members` members (100 000 by default), so that one runaway key cannot make every command on it slow or fill the memory. A command that would pass it changes nothing and answers:

  ```text
  (error) ERR the set 'tags' would hold 100001 members, more than resp.max_collection_members (100000). Every command reads and rewrites the whole value: split it over several keys, or raise the limit
  ```

  The limit applies immediately (`config set max_collection_members 5000`); `0` removes it. A key already above a lowered limit can still shrink. Hashes are documents and are not counted.

### 23.9 Configuration

| Setting | Default | |
| :--- | :--- | :--- |
| `resp.enabled` | `true` | Start the Redis listener with `kivadb start` / `serve` |
| `resp.port` | `6379` | Its port; also `--resp-port`, `KIVADB_RESP_PORT` |
| `resp.max_collection_members` | `100000` | Members a set, sorted set or list may hold ([23.8](#238-sets-and-sorted-sets)); `0` = no limit |
| `tls.*` | | Encrypted connections, see [16.11](#1611-encrypted-connections-tls) |

`--no-resp` turns it off for one start. If a real Redis already uses port 6379 on the machine, KivaDB says so in its log and carries on without the listener; choose another port.

### 23.10 Vector search and the semantic cache from a Redis client

An application that already talks to KivaDB through a Redis library can use the vector search of [Chapter 12](#chapter-12--vector-search-for-ai) and the semantic cache of [12.8](#128-semantic-cache-reuse-answers-to-the-same-question) on the same connection, with two commands of its own:

```text
KIVA.SIMILAR <pattern>.<field> <vector as JSON> [FILTER <field> <op> <value>]... [LIMIT n] [METRIC cosine|dot|l2] [EF n] [EXACT]
KIVA.API <request as JSON>
```

```text
127.0.0.1:6379> KIVA.SIMILAR doc:*.embedding "[1, 0, 0]" LIMIT 2
1) 1) "doc:1"
   2) "0.9938837"
   3) "{\"title\":\"Kinshasa guide\",\"embedding\":[0.9,0.1,0.0]}"
2) 1) "doc:2"
   2) "0.11043153"
   3) "{\"title\":\"Goma guide\",\"embedding\":[0.1,0.9,0.0]}"
```

* `KIVA.SIMILAR` answers one `[key, score, document]` triple per result, closest first. The score is a double in RESP3 and text in RESP2. It uses the vector index (exact or HNSW) when there is one, as `get similar` does. Each `FILTER` adds a condition the documents must pass, as `where` does ([12.9](#129-filtering-on-other-fields)): `FILTER lang = fr FILTER year >= 2020`. The value is a number, `true`, `false` or `null` when it reads as one, otherwise text.
* `KIVA.API` runs any structured request of [20.4](#204-structured-requests-for-tools) and answers its JSON result as text: `similar`, `query`, `semantic` (lookup and store), `stats`...

With redis-py, for a cache of LLM answers:

```python
import json, redis
r = redis.Redis(port=6379, username="kiva", password="...")
def ask(question, embedding):
    hit = json.loads(r.execute_command("KIVA.API", json.dumps(
        {"op": "semantic", "cache": "llm", "text": question, "vector": embedding})))
    if hit.get("hit"):
        return hit["answer"]
    answer = call_the_llm(question)
    r.execute_command("KIVA.API", json.dumps({"op": "semantic", "action": "store", "cache": "llm",
        "text": question, "vector": embedding, "answer": answer, "depends": ["doc:*"]}))
    return answer
```

Both commands are checked against the user's role like any other.

### 23.11 JSON documents (RedisJSON commands)

A Redis client can write and change KivaDB documents with the commands of RedisJSON, the JSON module of Redis. Libraries that support it work unchanged: `client.json.set(...)` in node-redis, `r.json().set(...)` in redis-py, `call("JSON.SET", ...)` in ioredis.

```text
127.0.0.1:6379> JSON.SET user:1 $ '{"name":"Amani","age":30,"embedding":[0.9,0.1,0]}'
OK
127.0.0.1:6379> JSON.NUMINCRBY user:1 $.age 1
"[31]"
127.0.0.1:6379> JSON.GET user:1 $.name
"[\"Amani\"]"
```

What is written this way is an ordinary KivaDB document, so **the same data is reachable through every model**:

```text
127.0.0.1:6379> HGET user:1 name
"Amani"
127.0.0.1:6379> KIVA.SIMILAR user:*.embedding "[1,0,0]" LIMIT 1
1) 1) "user:1"
   ...

kiva> get where user:*.age > 30
user:1: {"name":"Amani","age":31,"embedding":[0.9,0.1,0]}
```

Secondary indexes ([11.4](#114-indexes--making-queries-fast)) and vector indexes ([Chapter 12](#chapter-12--vector-search-for-ai)) cover these documents like any other, and a document written from the shell, the REST API or a driver can be read with `JSON.GET`.

* **Paths** name one value: `$` or `.` for the whole document, `$.address.city`, `.tags[0]`, `$.tags[-1]`. A path starting with `$` answers a list of matches, as RedisJSON 2 does (`"[31]"`); the older form (`.age`) answers the value itself (`"31"`). Wildcards (`$.items[*]`), `..`, slices and filters, which name several values, are refused with an error.
* `JSON.SET` at a path creates the missing objects on the way (RedisJSON requires them to exist); a new key must be created at the root. An existing key keeps its expiry.
* In RESP3, `JSON.NUMINCRBY` answers numbers and `JSON.TYPE` one more level of array, as RedisJSON does; node-redis is checked in both.
* Inside `MULTI` or a script, `JSON.*` commands are all or nothing with the rest: a script that fails after `JSON.SET` leaves the document as it was.
* A Redis string read with `JSON.GET` is a JSON string. `JSON.GET` options `INDENT`, `NEWLINE` and `SPACE` are accepted; the answer stays compact.

---

### 23.12 Blocking commands and WATCH

**Waiting for a job.** `BLPOP` and the other blocking commands take a value when there is one, and otherwise wait until another client (or the shell, the REST API, a script) puts one there, up to their timeout in seconds (`0` waits forever, fractions such as `0.5` work):

```text
127.0.0.1:6379> BLPOP q:jobs q:urgent 30
(waits...)                                   other client: RPUSH q:urgent send-mail:42
1) "q:urgent"
2) "send-mail:42"
```

* Keys are tried in the order given. `BRPOP`, `BLMOVE` (reliable queue), `BRPOPLPUSH`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX` and `BZMPOP` work the same way, with the replies of Redis.
* A waiting client does not hold up the others: commands keep running one at a time for everyone else.
* When a value arrives, the waiting clients are woken together, and the first to get its turn takes it: not necessarily the one that waited longest, as Redis would choose. With several workers on one queue, each value still goes to exactly one of them.
* A client that disconnects while waiting stops waiting: a value pushed afterwards stays in the list for the others.
* Inside `MULTI` or a script, a blocking command does not wait: it answers at once, with nil when there is nothing, as in Redis.
* `WAIT numreplicas timeout` waits until that many replicas ([Chapter 25](#chapter-25--replication)) have every write made so far, up to `timeout` milliseconds, and answers how many do.

**Optimistic locking with WATCH.** `WATCH` names keys; the next `EXEC` then runs only if none of them changed in between, and answers nil otherwise, without applying anything:

```text
127.0.0.1:6379> WATCH stock:42
OK
127.0.0.1:6379> GET stock:42
"5"
                                             other client: DECR stock:42
127.0.0.1:6379> MULTI
OK
127.0.0.1:6379> SET stock:42 4
QUEUED
127.0.0.1:6379> EXEC
(nil)
```

* A key counts as changed when it is written, deleted, created, emptied with its database (`FLUSHDB`), or expires, whoever does it: another Redis client, the KivaDB shell, the REST API, a script. The check is made with the writes of `EXEC`, at once: nothing can slip in between.
* `EXEC` and `DISCARD` end the watch; `UNWATCH` ends it without a transaction. `WATCH` inside `MULTI` is refused, as in Redis.
* Libraries use it as they do with Redis: `client.watch(...)` then `multi().exec()` in node-redis (a `WatchError` when a key changed), `watch` then `multi().exec()` answering `null` in ioredis.
* `WATCH` keeps track of deletions until `EXEC`: a watch left open for hours keeps that small record growing. Watch just before the transaction, as Redis applications do.

### 23.13 Streams

A **stream** is a log of entries, each with an id (`<milliseconds>-<sequence>`, increasing) and field-value pairs: events of an application, a queue that several workers share, a history that consumers read at their own pace.

```text
127.0.0.1:6379> XADD events MAXLEN ~ 10000 * type login user amani
"1793001014470-0"
127.0.0.1:6379> XRANGE events - +
1) 1) "1793001014470-0"
   2) 1) "type"
      2) "login"
      3) "user"
      4) "amani"
127.0.0.1:6379> XREAD BLOCK 5000 STREAMS events $
(waits for the next entry)
```

**Consumer groups** share a stream among workers: each entry goes to one consumer of the group, stays *pending* until that consumer acknowledges it, and can be taken over by another one when its consumer stopped:

```text
127.0.0.1:6379> XGROUP CREATE jobs workers $ MKSTREAM
OK
127.0.0.1:6379> XREADGROUP GROUP workers w1 COUNT 10 BLOCK 5000 STREAMS jobs >
...
127.0.0.1:6379> XACK jobs workers 1793001014470-0
(integer) 1
127.0.0.1:6379> XAUTOCLAIM jobs workers w2 60000 0 COUNT 10
```

* **Commands**: `XADD` (`*`, `<ms>-*` or an explicit id; `NOMKSTREAM`; trimming with `MAXLEN` or `MINID`), `XLEN`, `XRANGE` and `XREVRANGE` (`-`, `+`, a time alone, `(` to exclude, `COUNT`), `XDEL`, `XTRIM`, `XSETID`, `XREAD` (`COUNT`, `BLOCK`, `$`), `XGROUP CREATE | SETID | DESTROY | CREATECONSUMER | DELCONSUMER`, `XREADGROUP` (`>` for new entries, an id for the consumer's own pending ones; `NOACK`, `BLOCK`), `XACK`, `XPENDING` (summary or detailed, `IDLE`, per consumer), `XCLAIM` (`IDLE`, `TIME`, `RETRYCOUNT`, `FORCE`, `JUSTID`), `XAUTOCLAIM`, `XINFO STREAM | GROUPS | CONSUMERS`. Checked with node-redis (RESP3) and ioredis.
* **Storage.** A stream is a KivaDB object, like a set: written to disk, replicated, encrypted, readable from the shell (`get events`). `TYPE` answers `stream`, and other types' commands on it answer `WRONGTYPE`.
* **Size.** Each command reads and rewrites the whole stream, so a stream may hold at most `resp.max_collection_members` entries (100 000 by default, [23.8](#238-sets-and-sorted-sets)). Keep it trimmed as Redis applications usually do: `XADD ... MAXLEN ~ 10000`, or `XTRIM`. `~` trims exactly to the number given, which Redis allows (it may keep a few more).
* `XREAD` and `XREADGROUP` with `BLOCK` wait like `BLPOP` ([23.12](#2312-blocking-commands-and-watch)); `$` means the last id at the moment the command starts.

### 23.14 Unique indexes and `add` from a Redis client

A unique index ([11.7](#117-unique-indexes)) guards the documents a Redis client writes as well: `JSON.SET` and the other `JSON.*` commands that change a document, and `HSET` on a hash (a hash is a document whose values are text). A string written with `SET` is not a document and is never constrained.

A refused write answers an error whose first word is `UNIQUE`, the way Redis errors start with a code (`WRONGTYPE`, `READONLY`):

```text
127.0.0.1:6379> JSON.SET place:3 $ '{"type": "QUARTIER", "parent": "place:1", "name": "KATINDO"}'
(error) UNIQUE 'place:3' not written: 'place:2' already holds ["quartier","place:1","katindo"] in the unique index on place:* (type, parent, name) ignore case
```

With ioredis, test that word to answer a conflict:

```js
try {
  await redis.call("JSON.SET", key, "$", JSON.stringify(place));
} catch (err) {
  if (err.message.startsWith("UNIQUE ")) return res.status(409).json({ error: "This place already exists." });
  throw err;
}
```

**In `MULTI` / `EXEC`.** The commands are queued and answer `QUEUED`, as in Redis; the unique indexes are checked when `EXEC` applies them, all together. If one write would leave a duplicate, `EXEC` answers the `UNIQUE` error and **nothing** of the transaction is applied: not the other commands either. (A command refused while it is queued, such as an unknown command, makes `EXEC` answer `EXECABORT` instead; a watched key that changed makes it answer nil, [23.12](#2312-blocking-commands-and-watch).)

```text
127.0.0.1:6379> MULTI
OK
127.0.0.1:6379> SET counter 1
QUEUED
127.0.0.1:6379> JSON.SET place:5 $ '{"type": "VILLE", "parent": null, "name": "goma"}'
QUEUED
127.0.0.1:6379> EXEC
(error) UNIQUE 'place:5' not written: 'place:1' already holds ["ville",null,"goma"] in the unique index on place:* (type, parent, name) ignore case
127.0.0.1:6379> EXISTS counter
(integer) 0
```

**`add`** (a key named by KivaDB, [6.7](#67-add--let-kivadb-name-the-key)) goes through `KIVA.API`, which answers the key and the identifier as JSON:

```js
const r = JSON.parse(await redis.call("KIVA.API", JSON.stringify({
  op: "add", prefix: "place:", value: { type: "VILLE", parent: null, name: "Bukavu" },
})));
// r = { key: "place:6adf9c3ca32f23553b1dbb10", id: "6adf9c3ca32f23553b1dbb10", type: "object" }
```

The queries with `in`, `like` and `ilike` go through `KIVA.API` too: `{"op": "query", "target": "place:*", "where": {"parent": ["place:1", "place:2"]}}` ([20.4](#204-structured-requests-for-tools)).

## Chapter 24 — Connecting from your programs

Start KivaDB (`kivadb start`), then pick the approach that suits your language.

| Approach | Good for | Needs |
| :--- | :--- | :--- |
| REST API (port 8080) | Any language, web applications, quick integration | An HTTP library |
| Official driver (port 2911) | Node.js, Python and Rust applications | The driver ([24.3](#243-nodejs), [24.4](#244-python), [24.5](#245-rust)) |
| Binary protocol (port 2911) | Lower overhead, long-lived connections, notifications, from any language | A few lines of socket code |
| Redis protocol (port 6379) | Languages that have a Redis library but no KivaDB driver yet (Go, PHP, Java...) | A Redis library |
| Command line (`kivadb -e`) | Shell scripts, batch files | Nothing |

| Language | Recommended | Section |
| :--- | :--- | :--- |
| JavaScript / TypeScript (Node.js) | The `kivadb` driver | [24.3](#243-nodejs) |
| Python | The `kivadb` driver | [24.4](#244-python) |
| Rust | The client in the `kivadb` crate | [24.5](#245-rust) |
| Go, Java, PHP, C#... | A Redis library, or the REST API | [24.6](#246-redis-libraries), [24.8](#248-any-other-language) |
| Shell scripts | `kivadb -e` or curl | [24.1](#241-command-line), [24.2](#242-curl) |

### 24.1 Command line

```bash
kivadb -e "set backup_done true" -e "get backup_done"
```

Check the exit code: `0` success, `1` at least one error.

### 24.2 curl

See [Chapter 21](#chapter-21--the-rest-api). Summary:

```bash
curl -X POST http://localhost:8080/v1/key/city:1 -d '{"name": "Kinshasa"}'
curl http://localhost:8080/v1/key/city:1
curl -X DELETE http://localhost:8080/v1/key/city:1
```

### 24.3 Node.js

**With the REST API** (Node 18 or later, nothing to install):

```javascript
async function main() {
  const base = "http://127.0.0.1:8080";
  await fetch(`${base}/v1/key/city:1`, {
    method: "POST",
    body: JSON.stringify({ value: { name: "Kinshasa" }, ttl: 3600 }),
  });
  const reply = await (await fetch(`${base}/v1/key/city:1`)).json();
  console.log(reply.status, reply.type, reply.data.name);   // ok object Kinshasa
}
main();
```

**With the official driver.** The npm package `@fomadev/kivadb` ([`drivers/node`](drivers/node)) speaks the binary protocol, has no dependencies and ships TypeScript types: `npm install @fomadev/kivadb`. Its version numbers are its own, separate from the server's ([its README](drivers/node/README.md)):

```javascript
const { connect } = require("@fomadev/kivadb");

const db = await connect({ host: "localhost", port: 2911, user: "kiva", password: "" });
await db.set("user:1", { name: "Alice", role: "admin" }, { ttl: 3600 });
console.log(await db.get("user:1"));
console.log(await db.similar("doc:*.embedding", [0.1, 0.2, 0.3], { limit: 5 }));
db.on("change", (e) => console.log(e.event, e.key));
await db.subscribe("user:*");
```

To encrypt the connection, add `tls: { fingerprint: "..." }` with the fingerprint shown by `kivadb tls` ([16.11](#1611-encrypted-connections-tls)). Its README lists every method.

From driver **1.2.0** (with KivaDB 2.0 or later), a JavaScript `Date` is stored as a `datetime` and comes back as a `Date`; a `date` or a `time` is given as text with its type: `db.set("born", "2004-05-09", { type: "date" })` ([5.8](#58-date-datetime-time)).

From driver **1.1.0** (with KivaDB 1.7.0 or later), `add` stores a document under a key KivaDB names ([6.7](#67-add--let-kivadb-name-the-key)), `query` takes a condition as JSON with `in`, `like` and `ilike` ([11.1](#111-the-condition), [20.4](#204-structured-requests-for-tools)), and an error keeps the code word of KivaDB's message in `err.code`, such as `UNIQUE` for a write a unique index refuses ([11.7](#117-unique-indexes)):

```javascript
const { key, id } = await db.add("place:", { type: "QUARTIER", parent: "place:1", name: "Katindo" });
const quarters = await db.query("place:*", { type: "QUARTIER", parent: "place:1" });
const found = await db.query("place:*", { field: "name", cmp: "ilike", value: "kat%" });
const children = await db.query("place:*", { parent: ["place:1", "place:2"] });   // a list means in
try {
  await db.add("place:", { type: "QUARTIER", parent: "place:1", name: "KATINDO" });
} catch (err) {
  if (err.code === "UNIQUE") console.log("already there");   // with an index ... ignore case
}
```

Driver 1.0.0 has neither `add` nor `query`; its `api()` sends the same structured requests. **A minimal client to read**: [`examples/nodejs/kiva.js`](examples/nodejs/kiva.js) shows the protocol in about fifty lines:

```javascript
const { KivaClient } = require("./kiva");

async function main() {
  const db = new KivaClient("127.0.0.1", 2911);
  db.onEvent = (event) => console.log("event:", event);
  // await db.auth("fordi", "mon_pass");      // when authentication is on

  await db.exec('set product:1 {"name": "Pen", "price": 2}');
  console.log(await db.exec("get product:1.name"));
  console.log(await db.exec("count where product:*.price < 10"));

  await db.exec("subscribe product:*");
  await db.exec("json set product:1 .price 3");
  try {
    await db.exec('update product:1 "not an object"');
  } catch (err) {
    console.log("refused:", err.message);
  }
  await db.exec("del product:1");
  setTimeout(() => db.close(), 200);
}

main();
```

Output:

```text
[ 'product:1.name: Pen [string]' ]
[ 'Count: 1' ]
event: update product:1 (object)
refused: Error: Cannot update [object] with a value formatted as [string].
event: del product:1
```

`exec` sends any command of this manual and returns the output lines; it throws when the command reports an error.

### 24.4 Python

**With the official driver.** The package `kivadb` ([`drivers/python`](drivers/python)) speaks the binary protocol, needs only the standard library and is typed. It needs Python 3.10 or later. It is not on PyPI yet: `pip install path/to/kiva-db/drivers/python`. Its version numbers are its own, separate from the server's ([its README](drivers/python/README.md)):

```python
import datetime as dt
import kivadb

with kivadb.connect(host="localhost", port=2911, user="kiva", password="") as db:
    db.set("user:1", {"name": "Ana", "role": "dev"}, ttl=3600)
    db.set("user:1:seen", dt.datetime.now(dt.timezone.utc))      # a datetime
    print(db.get("user:1"))
    print(db.query("user:*", {"role": "dev"}, order_by="name", limit=20))
    with db.transaction():
        db.set("acct:a", 10)
        db.set("acct:b", 20)
```

`datetime.datetime` (with a zone), `datetime.date` and `datetime.time` are stored as KivaDB's `datetime`, `date` and `time`, and come back as such. A datetime without a zone is refused rather than guessed. A lost connection fails the request that was being sent, which is never sent again, and the next request reconnects. The README lists the methods, the errors and these rules.

**With the REST API** and the standard library only:

```python
import json
import urllib.request

BASE = "http://127.0.0.1:8080"

def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(BASE + path, data=data, method=method)
    with urllib.request.urlopen(request) as response:
        return json.loads(response.read())

call("POST", "/v1/key/city:1", {"value": {"name": "Kinshasa"}, "ttl": 3600})
print(call("GET", "/v1/key/city:1")["data"]["name"])          # Kinshasa
print(call("POST", "/v1/exec", {"command": "keys limit 5"})["output"])
call("DELETE", "/v1/key/city:1")
```

When authentication is on, add the header `Authorization: Basic <base64 of "user:password">` to each request. Note that `urlopen` raises an exception for answers such as 404. The `redis` package works too, through the Redis protocol ([24.6](#246-redis-libraries)).

### 24.5 Rust

Add KivaDB as a dependency of your project, in `Cargo.toml`:

```toml
[dependencies]
kivadb = { git = "https://github.com/fomadev/kiva-db", tag = "v1.7.1" }
```

Then use its client:

```rust
use kivadb::client::Client;
use kivadb::Value;

fn main() -> Result<(), kivadb::KivaError> {
    let mut db = Client::connect("127.0.0.1:2911")?;
    // db.auth("fordi", "mon_pass")?;        // when authentication is on

    db.set("greeting", &Value::Str("hello".into()), 0)?;
    db.set("session", &Value::Str("abc123".into()), 3600)?;   // expires in an hour
    assert_eq!(db.get("greeting")?, Some(Value::Str("hello".into())));
    assert_eq!(db.get("ghost")?, None);

    println!("{}", db.exec("count where user:*.role = \"admin\"")?);
    db.del("greeting")?;
    Ok(())
}
```

| Method | Does |
| :--- | :--- |
| `Client::connect(addr)` | Opens a connection |
| `auth(user, password)` | Logs in |
| `get(key)` | Returns `Option<Value>` |
| `set(key, &value, ttl_secs)` | Creates or replaces; ttl 0 = no expiry |
| `del(key)` | Returns whether the key existed |
| `exec(command)` | Runs any command; returns its output, or an error |
| `ping()` | Checks the connection |
| `events` | Notifications received so far, after a `subscribe` |

### 24.6 Redis libraries

Any Redis client library works for the commands of [Chapter 23](#chapter-23--redis-compatibility): point it at port 6379. This is often the fastest way to use KivaDB from a language that has no KivaDB driver yet (Go's `go-redis`, PHP's `phpredis`, Java's `Jedis`...), or from Python with `redis`.

### 24.7 Kiva Studio, the graphical client

Kiva Studio is the desktop application for KivaDB (Windows, Linux, macOS). It is a separate project, [fomadev/kiva-studio](https://github.com/fomadev/kiva-studio), and connects to port 2911 like any other client: enter the host, the port, a user name and a password. It offers a data browser and editor (with a tree view for documents), a visual query builder that shows the execution plan and creates the missing index in one click, vector similarity search, a command console, an activity feed with alerts (fragmentation, damaged data, replication, failed logins), user management for administrators, a health dashboard, and one-click switching between remembered servers. Its README explains how to build and run it.

### 24.8 Any other language

Two options: use the REST API with your language's HTTP library, or implement the binary protocol of [section 20.2](#202-the-binary-protocol) (with the structured requests of [section 20.4](#204-structured-requests-for-tools) if you want JSON instead of text). The protocol is small; the Node.js client is about fifty lines and is a good model: build a frame, send it, read frames until one is not a notification.

### 24.9 Coming from MongoDB

A MongoDB collection becomes a key pattern: each document is stored under `<collection>:<_id>`, and the operations an application uses most have a direct counterpart.

| MongoDB | KivaDB |
| :--- | :--- |
| `db.places.insertOne(doc)` | `add place: {...}`, or `set place:<id> {...}` to keep an identifier ([6.7](#67-add--let-kivadb-name-the-key)) |
| `ObjectId()` | The identifier `add` makes: the same 24-character format, sorted by time |
| `findOne({_id})` | `get place:<id>` |
| `find({type: "VILLE", parentId: p})` | `get where place:*.type = "VILLE" and parentId = "p"` ([11.2](#112-get-where--find-documents)) |
| `$in`, `$nin` | `in [...]`, `not in [...]` |
| `$ne`, `$gt`, `$gte`, `$lt`, `$lte` | `!=`, `>`, `>=`, `<`, `<=` |
| `$or`, `$and`, `$not` | `or`, `and`, `not`, parentheses |
| `$regex: /^kin/i` | `ilike "kin%"`; `$regex: /^Kin/` is `like "Kin%"`. Other regular expressions have no counterpart |
| `countDocuments`, `$group` with `$sum`, `$avg`, `$min`, `$max` | `count`, `sum`, `avg`, `min`, `max`, `group by` ([11.3](#113-aggregates-count-sum-avg-min-max-group-by)) |
| `createIndex({type: 1})` | `create index on place: (type)` |
| `createIndex({type: 1, parentId: 1, name: 1}, {unique: true, collation: {strength: 2}})` | `create unique index on place: (type, parentId, name) ignore case` ([11.7](#117-unique-indexes)) |
| `updateOne({_id}, {$set: {name}})` | `json set place:<id> .name "..."` ([10.3](#103-changing-a-field)) |
| `deleteOne({_id})`, `deleteMany({parentId: {$in: ids}})` | `del place:<id>`; for many, read the keys with `get where ... in [...]`, then `del a and b ...` in one transaction ([13](#chapter-13--transactions)) |
| Sessions and transactions | `begin` ... `commit` ([13](#chapter-13--transactions)) |
| `find(...).sort({name: -1}).limit(20)` | `get where ... order by name desc limit 20`; the next page with `after <last key>` rather than `skip` ([11.2](#112-get-where--find-documents)) |
| `$lookup` | Not part of the language: related data is read by name ([11.6](#116-what-a-query-does-not-do)) |

**A missing field in a unique index.** MongoDB counts a field a document lacks as `null`: two top-level places without `parentId`, of the same type and name, conflict. KivaDB does not constrain a document that lacks one of the fields of a unique index ([11.7](#117-unique-indexes)), so those two would both be written. Write the field as `null` instead of leaving it out, in the application and in the import: `{"type": "PROVINCE", "parentId": null, "name": "Nord-Kivu"}` is checked, `{"type": "PROVINCE", "name": "Nord-Kivu"}` is not. In the import script below, add `doc.parentId ??= null;` before `console.log` for each field of a unique index that documents may lack.

**Copying a collection.** Export it as one JSON document per line, then write each one under its `_id` in a `.kva` file and run it with `kivadb run` ([3.10](#310-command-files-kva)):

```bash
mongoexport --db app --collection places --jsonFormat=relaxed --out places.json
node -e '
  // MongoDB Extended JSON -> plain JSON: identifiers and dates become text, numbers become
  // numbers (an integer too large to stay exact in JavaScript is kept as text).
  const plain = (k, v) => {
    if (v === null || typeof v !== "object") return v;
    if ("$oid" in v) return v.$oid;
    if ("$date" in v) return typeof v.$date === "string" ? v.$date : new Date(v.$date).toISOString();
    if ("$numberLong" in v) return Number.isSafeInteger(Number(v.$numberLong)) ? Number(v.$numberLong) : v.$numberLong;
    for (const t of ["$numberInt", "$numberDouble", "$numberDecimal"]) if (t in v) return Number(v[t]);
    return v;
  };
  const lines = require("fs").readFileSync("places.json", "utf8").split("\n").filter(Boolean);
  for (const line of lines) {
    const { _id, ...doc } = JSON.parse(line, plain);
    console.log(`set place:${_id} ${JSON.stringify(doc)}`);
  }' > places.kva
kivadb run places.kva
```

```text
OK: place:66f1c2a0e4b0a1b2c3d4e5f6 saved.
OK: place:66f1c2a0e4b0a1b2c3d4e5f7 saved.
```

The identifiers keep their format, so the applications that read them do not change: a `parentId` that held an `ObjectId` now holds its 24 characters, and `get where place:*.parentId = "66f1c2a0e4b0a1b2c3d4e5f6"` finds the children. Dates become text such as `"2024-09-23T10:00:00Z"` (a date before 1970 or after 9999, which `mongoexport` writes as a number of milliseconds, is turned into the same form), which `>` and `<` compare as moments when the value is one (`created_at > now() - 30 days`, `created_at >= datetime "2026-01-01"`, [11.1](#111-the-condition)), and as text otherwise, correctly as long as they all have this form. `$numberInt`, `$numberLong`, `$numberDouble` and `$numberDecimal` become numbers, with two limits: an integer beyond 2^53 (9 007 199 254 740 992) would lose its last digits, so it stays text; a `Decimal128` keeps about 15 significant digits, enough for measures, not for amounts that must be exact to the last digit (store those as text, or in the smallest unit as an integer). Texts with quotes are safe: `JSON.stringify` writes `\"`, and a document literal is read as JSON (the shell's limit on quotes concerns quoted strings such as `set name "..."`, [4](#chapter-4--syntax-rules)). Type the `node -e` command in bash or Git Bash: Windows PowerShell 5.1 changes the double quotes it passes to programs. The `.kva` file itself may come from any Windows tool: since 1.7.1, `kivadb run` reads UTF-16 and byte-order-marked files ([3.10](#310-command-files-kva)). Create the indexes and the unique indexes before the import: a duplicate already in MongoDB is then refused, with the key that holds the value, instead of being copied (`kivadb run places.kva --keep-going` goes on with the other lines).

---

## Chapter 25 — Replication

### 25.1 What it is

Replication keeps a live copy of a database on another KivaDB. The one that receives the writes is the **master**; the copies are **replicas**. Uses:

* **Safety**: if the master's disk dies, a replica has the data.
* **Read load**: applications can read from replicas.
* **Standby**: a replica can be promoted to master.

Replication covers **every workspace**, and the users, stored scripts and triggers they share ([25.9](#259-what-is-replicated)).

### 25.2 Setting it up

You need two running KivaDB, usually on two machines. On the future master, nothing special is required beyond being reachable from the replica ([16.7](#167-opening-kivadb-to-the-network)).

On the replica, tell it whom to follow:

```text
kiva> replica of 192.168.1.10 2911
Now replicating from 192.168.1.10:2911. Local data is replaced by the master's.
```

What happens then:

1. The replica **replaces its own data with the master's**:
   * every workspace of the master is emptied on the replica, or created there if it does not exist yet;
   * the replica's users, stored scripts and triggers are replaced by the master's;
   * the replica's workspaces that the master does not have are unregistered, as `workspace remove` does: their files stay on disk.
2. The master sends a full copy (a snapshot) of all of it.
3. The master then sends every change as it happens, whatever the workspace.

The choice is remembered in the configuration: after a restart, the replica resumes following its master. If the connection drops, the replica retries every two seconds and resynchronises by itself.

If the master requires authentication, give the replica an admin account to log in with:

```text
kiva> config set master_user "kiva"
kiva> config set master_password "the-master-password"
```

### 25.3 A replica is read-only

```text
kiva> set nope 1
Error: READONLY: this node is a replica; write to the master or run 'replica promote'
```

This holds in every workspace. Triggers do not run on a replica: what the master's triggers write reaches it with the rest, and running them again would apply their effects twice.

### 25.4 Watching it

On the master:

```text
kiva> replica status
Role: master
  Mode: async (min acks: 1)
  Sequence: 1024
  Workspaces: 3 (default, hr, sales)
  Connected replicas: 1
    #7 192.168.1.20:51344 (listens on 192.168.1.20:2911) lag: 0 ops, connected 2026-10-12 09:50:01
```

*Lag* is how many operations the replica has not confirmed yet; 0 means it is up to date.

On the replica:

```text
kiva> replica status
Role: replica (read-only)
  Master: 192.168.1.10:2911
  Link: connected
  Applied sequence: 1024 (318 operations since connect)
  Last sync: 2026-10-12 09:50:01
  Workspaces: 3 (default, hr, sales)
```

The sequence counts the operations of all workspaces together, in one order: a replica that acknowledged operation 1024 has everything before it, in every workspace. `wait` and the lag mean the same.

**Clocks.** Each heartbeat carries the master's time, and the replica compares it with its own. `stats` on a replica shows the difference (`> Clock : 3 ms ahead of the master`). Beyond one second, the replica warns once in its log and in the audit log (`this node's clock is 4.2 s behind the master's ...`): TTLs, `now()` and certificates then disagree between the nodes, and a promoted replica would write dates off by that much. KivaDB does not move its clock: turn on the system's time synchronization (NTP) on every node.

### 25.5 Asynchronous or synchronous

* **Asynchronous** (default). The master answers as soon as it has written locally and sends the change to replicas right after. Fastest; if the master dies at the wrong instant, the very last writes may not have reached a replica.
* **Synchronous.** The master waits until `min_slaves_ack` replicas have confirmed each write before answering.

```text
kiva> config set sync_mode sync
kiva> config set min_slaves_ack 1
```

If the confirmations do not arrive within five seconds, the write is kept and a warning is shown:

```text
Warning: write acknowledged by 0 of 1 required replica(s).
```

In asynchronous mode you can still wait at chosen moments with `wait`:

```text
kiva> set important "data"
OK: important saved.
kiva> wait 1
OK: 1 replica(s) acknowledged.
```

`wait <N> [timeout_ms]` blocks until N replicas have everything written so far (default timeout: 5000 ms).

### 25.6 Commands

| Command | Where | Effect |
| :--- | :--- | :--- |
| `replica of <host> <port>` | replica | Start following that master |
| `replica of none` | replica | Stop and become a master |
| `replica stop` | replica | Pause following (stays read-only) |
| `replica start` | replica | Follow the configured master again |
| `replica sync` | replica | Force a full resynchronisation |
| `replica promote` | replica | Become a writable master |
| `replica status` | both | Show the state |
| `wait <N> [timeout_ms]` | master | Wait for N replicas to be up to date |
| `failover to <replica>` | master | Hand over the master role |
| `failover status` | both | Automatic failover: settings, epoch, known nodes, last event |

### 25.7 Failover

**Planned** - the master is alive and you want to switch roles (maintenance, migration). On the master, name the replica by its number from `replica status`:

```text
kiva> failover to 7
Failover complete: 192.168.1.20:2911 is the new master and this node now follows it.
Other replicas must be pointed at the new master with: replica of <host> <port>
```

The master waits for the replica to be up to date, tells it to become master, and becomes its replica.

**Unplanned** - the master is dead. With automatic failover on, the replicas elect a new master by themselves ([25.8](#258-automatic-failover)). Otherwise, on the replica you choose as the new master:

```text
kiva> replica promote
This node is now a master.
```

Then point your applications, and the other replicas, to it (`replica of <host> <port>`).

### 25.8 Automatic failover

With **`failover.auto`** on, the replicas watch the master and, when it dies, choose one of them as the new master, without anybody at the keyboard.

**Setting it up.** Three nodes at least: one master and two replicas (see why below). On **every** node:

```text
kiva> config set failover.auto true
```

Every node logs in to the others with the replication account (`config set master_user ...` and `master_password ...`, [25.2](#252-setting-it-up)): use the same administrator account and password on all of them. Once the replicas follow the master, its accounts are theirs ([25.9](#259-what-is-replicated)). The replicas must all name the master with the same address (the same IP, or the same host name).

```text
kiva> failover status
Automatic failover: on (heartbeat 1000 ms, master down after 5000 ms of silence, 2 of 3 nodes must agree)
Epoch: 0
This node (10.0.0.12:2911): replica of 10.0.0.11:2911 (applied sequence 18342, last heard 0.4 s ago)
Known nodes: 10.0.0.13:2911, 10.0.0.11:2911
```

**How it works.**

1. **Heartbeats.** Every `failover.heartbeat_ms`, the master sends each replica a heartbeat with the list of the group's nodes: every replica knows the others.
2. **Suspicion.** A replica that hears nothing for `failover.down_after_ms` tries to reach the master itself, then asks the other replicas whether they lost it too.
3. **Agreement.** Only when a **majority** of the group (the master counted) sees the master down does anything happen. This is what prevents two masters: a replica cut off from the master by a network problem, while the others still see it, is alone and changes nothing.
4. **Election.** The replica that received the most of the master's changes is chosen (then the highest `failover.priority`, then the smallest address), so that as little as possible is lost. It asks the others for their vote in a new **epoch**; each node votes once per epoch, and only for a candidate at least as up to date as itself. With a majority of votes, it becomes the master.
5. **Resynchronisation.** The new master tells the other replicas, which follow it and resynchronise.
6. **The old master comes back.** It finds that the group moved to a newer epoch, becomes a replica of the new master and resynchronises from it.

With the defaults, a new master is in place a few seconds after the old one stopped answering (`down_after_ms`, plus the time to ask the others). `failover status` shows the last event, for example `promoted to master in epoch 1 (2 of 3 nodes agreed that 10.0.0.11:2911 was down)`; the audit log keeps every promotion.

**Why three nodes.** With a master and one replica, the replica alone is not a majority of two: if it promoted itself every time it lost sight of the master, a simple network cut would leave two masters accepting different writes. So it does not, and `failover status` says why (`only 1 of the 2 nodes needed see it down`). `config set failover.quorum 1` allows it anyway, if you accept that risk.

**Clients.** A replica refuses writes and names the master: `READONLY: this node is a replica of 10.0.0.12:2911`. Redis clients find it in `INFO replication` (`master_host`, `master_port`). Give your application the addresses of all the nodes and let it move to the master named in the error, or reconnect to the next address when a node does not answer.

**What can be lost.** Replication is asynchronous by default: writes the old master accepted in its last instants, before they reached any replica, are not on the new master. Writes a cut-off old master accepted before it learned that it was replaced are also lost when it resynchronises. Synchronous replication ([25.5](#255-asynchronous-or-synchronous)) narrows the first window.

**Choosing who may become master.** `failover.priority 0` on a node (a small machine, a backup site) means it never takes over by itself; between equally up-to-date replicas, a higher priority wins.

| Setting | Default | |
| :--- | :--- | :--- |
| `failover.auto` | `false` | Turn it on, on every node |
| `failover.heartbeat_ms` | `1000` | Interval of the master's heartbeats |
| `failover.down_after_ms` | `5000` | Silence before a replica suspects the master. Shorter reacts faster but risks a failover during a short network hiccup |
| `failover.quorum` | `0` | Nodes that must agree; `0` = a majority of the group |
| `failover.priority` | `100` | `0` = never become master by itself |

KivaDB keeps its group state in `[failover]` too (`node_id`, `epoch`, `voted`, `nodes`): leave those alone.

### 25.9 What is replicated

| Replicated | Not replicated |
| :--- | :--- |
| The keys of every workspace, with their TTL | The configuration (`kivadb.conf`, `config set`): set it on each node |
| The creation and removal of workspaces | Secondary and vector indexes: create them on each node (`create index` works on a replica), so that each one serves the queries it receives |
| Users, passwords, roles and grants | Pinned patterns (`memory pin`) and the value cache |
| Stored scripts and triggers | The semantic cache, and publish/subscribe messages |

* **Users.** A replica has the master's accounts: an account created, changed or removed on the master is the same on its replicas. That is what lets applications log in to the new master after a failover. The replica's own accounts are replaced when it starts following: keep the master's administrator password at hand.
* **One stream.** Every workspace travels on the same connection and the same sequence, in the order the master committed the operations. A replica therefore never sees a workspace further ahead than another.
* **Versions.** A node of KivaDB 2.x and a node of 1.x never follow each other: upgrade a group as a whole ([33.5](#335-a-replication-group)). Within 1.x, a master refuses a replica of a version older than 1.6.0, which knew only the default workspace: it would apply every workspace's changes to its default one.

---

## Chapter 26 — Lua scripting

For logic that needs several steps - read a value, decide, write another - you can run small programs **inside** KivaDB, written in [Lua](https://www.lua.org) 5.4. The steps run next to the data, without network round trips, and atomically.

### 26.1 `eval` — run code

```text
eval '<lua code>' [arguments...]
```

```text
kiva> eval 'return kiva.get("user:1").name'
Alice
kiva> eval 'return 6 * 7'
42
```

The code goes between single quotes so that double quotes can be used inside. What the script `return`s is displayed. A single expression is displayed even without `return` (that is why `kiva.set(...)` alone shows `true`). A script that returns nothing displays `OK`.

### 26.2 The `kiva` functions

| Function | Does |
| :--- | :--- |
| `kiva.get(key)` | Returns the value, or `nil` if the key does not exist |
| `kiva.set(key, value [, ttl])` | Creates or replaces the key; `ttl` in seconds |
| `kiva.del(key)` | Deletes; returns `true` if the key existed |
| `kiva.has(key)` | `true` or `false` |
| `kiva.typeof(key)` | The type name, or `nil` |
| `kiva.scan([pattern])` | A list of key names (all keys without pattern) |
| `kiva.incr(key [, amount])` | Adds to a number (1 by default), creating it at 0 if needed; returns the new value |

How values travel between KivaDB and Lua:

| KivaDB | Lua |
| :--- | :--- |
| string | string |
| number | number |
| boolean | boolean |
| null | `nil` |
| array | table with positions 1, 2, 3... |
| object | table with named fields |
| bytes | string |

```text
kiva> eval 'kiva.set("visits", (kiva.get("visits") or 0) + 1) return kiva.get("visits")'
1
kiva> eval 'kiva.set("made", {list = {1, 2, 3}, ok = true})'
true
kiva> get made.list
made.list: [1,2,3] [array]
```

Lua tables count from 1: `kiva.get("colors")[1]` is the first colour. An empty Lua table is stored as an empty object.

### 26.3 Arguments and print

Words after the code are available in the table `ARGV`, as text. `print` shows lines before the result.

```text
kiva> eval 'print("hello", ARGV[1], ARGV[2])' world 42
hello	world	42
OK
```

### 26.4 Scripts are atomic

Everything a script writes is committed together when it ends. If it fails, nothing is kept:

```text
kiva> eval 'kiva.set("half", 1); error("boom")'
Error: ScriptError: runtime error: [string "script"]:1: boom
kiva> has half
No: Key 'half' does not exist.
```

If another session modified one of the same keys while the script was running, the script fails with `ERR_TRANSACTION_CONFLICT` and can be run again. A script run inside your own `begin ... commit` joins your transaction.

### 26.5 The sandbox

Scripts cannot harm the machine:

* No access to files, the operating system or the network: `io`, `os`, `require`, `load`, `dofile` do not exist.
* A time limit (`scripting.timeout_ms`, 5 seconds): `Error: ScriptError: runtime error: script exceeded the time limit of 5000 ms`.
* A memory limit (`scripting.memory_limit_mb`, 64 MB).

Available Lua libraries: `string`, `table`, `math`, `utf8`. Running scripts requires the right to write (role `editor` or `admin`).

### 26.6 Stored scripts

Write the script in a file, for example `increment.lua`:

```lua
local key = ARGV[1]
local by = tonumber(ARGV[2] or "1")
return kiva.incr(key, by)
```

Load it once under a name, then call it by that name from anywhere:

```text
kiva> script load "C:\scripts\increment.lua" as "inc"
Script 'inc' stored (82 bytes).
kiva> script run "inc" visits 5
6
kiva> script list
inc                      82 bytes
```

| Command | Effect |
| :--- | :--- |
| `script load "<file>" as "<name>"` | Stores (or replaces) a script. Admin only |
| `script run "<name>" [arguments]` | Runs it |
| `script list` | Lists stored scripts |
| `script show "<name>"` | Displays the source |
| `script delete "<name>"` | Removes it. Admin only |

The file is read by the KivaDB process, on the machine where it runs: give a full path. A script with a syntax error is refused at load time. Stored scripts are shared by all workspaces.

### 26.7 Triggers

A **trigger** runs a stored script automatically when keys change.

```text
create trigger [name] on set|update|del "<pattern>" run "<script>"
```

| Event | Fires when |
| :--- | :--- |
| `set` | A matching key is created |
| `update` | A matching existing key is overwritten |
| `del` | A matching key is deleted |

The script receives `ARGV[1]` = the event and `ARGV[2]` = the key. Example: keep a journal of user creations. File `audit_log.lua`:

```lua
kiva.set("log:" .. ARGV[1] .. ":" .. ARGV[2], "seen")
```

```text
kiva> script load "C:\scripts\audit_log.lua" as "audit_log"
Script 'audit_log' stored (54 bytes).
kiva> create trigger watch_users on set "user:*" run "audit_log"
Trigger 'watch_users' created: on set "user:*" run "audit_log".
kiva> set user:9 "new"
OK: user:9 saved.
kiva> has log:set:user:9
Yes: Key 'log:set:user:9' exists.
kiva> triggers
watch_users      on set    "user:*" run "audit_log"
kiva> drop trigger watch_users
Trigger 'watch_users' dropped.
```

Triggers run just **after** the change, in the background: the command that caused them does not wait, and a failing trigger does not undo it (the failure is recorded in the audit log). Writes made by a trigger do not fire triggers, so they cannot loop.

### 26.8 Measuring and debugging

```text
kiva> bench script "inc" iterations 1000 counter
SCRIPT inc: 1000 ops in 0.284s -> 3521 ops/sec (avg 0.284 ms)
```

Starting KivaDB with `--lua-debug` makes every script show the lines it executes (`[lua-debug] line 3`), which helps to follow its path.

---

## Chapter 27 — Monitoring

### 27.1 Quick looks

* `stats` in a shell ([17.1](#171-stats--the-dashboard)).
* `GET /v1/stats` for the same figures as JSON.
* `GET /health` to check that the server answers (a liveness probe).
* `GET /ready` to check that it should receive traffic (a readiness probe): `503` while it stops, and on a replica that is not connected to its master, for instance while it receives its first snapshot. Neither needs a password.
* `kivadb doctor` for a health check of the home ([17.9](#179-kivadb-doctor--a-health-check-of-the-home)).
* `kivadb status` to check that the process is running.

### 27.2 Prometheus

`GET /metrics` on the REST port returns metrics in the Prometheus text format:

```text
# HELP kiva_requests_total Operations processed, by type.
# TYPE kiva_requests_total counter
kiva_requests_total{type="get"} 2
kiva_requests_total{type="set"} 1
kiva_requests_total{type="del"} 0
kiva_requests_total{type="other"} 0
# HELP kiva_keys Live keys in the workspace.
# TYPE kiva_keys gauge
kiva_keys 2
...
```

| Metric | Meaning |
| :--- | :--- |
| `kiva_requests_total{type=...}` | Operations processed: `get`, `set`, `del`, `other` |
| `kiva_request_duration_seconds` | Histogram of the time spent executing commands: `_bucket{le=...}` from 0.0001 s to 5 s, `_sum` and `_count` |
| `kiva_errors_total` | Commands that ended in an error |
| `kiva_http_requests_total` | HTTP requests received |
| `kiva_failed_logins_total` | Rejected logins |
| `kiva_keys` | Keys in the workspace |
| `kiva_storage_bytes` | Size of the data file |
| `kiva_storage_live_bytes` | Bytes of current data |
| `kiva_storage_fragmentation_ratio` | Dead share of the file, between 0 and 1 |
| `kiva_memory_bytes` | Estimated index memory |
| `kiva_checksum_errors_total` | Damaged records detected |
| `kiva_compactions_total` | Compactions done |
| `kiva_connected_clients` | Open TCP connections |
| `kiva_replicas_connected` | Replicas attached |
| `kiva_replication_lag_ops` | Operations the slowest replica is behind |
| `kiva_uptime_seconds` | Time since start |
| `kiva_changes_last_seq`, `kiva_changes_retained`, `kiva_changes_bytes` | The change journal, when the workspace keeps one ([22.5](#225-the-change-journal-changes-kept-read-again-from-a-cursor)) |
| `kiva_changes_consumer_lag{consumer=...}` | Changes a named consumer has not acknowledged yet |

The 95th percentile of the last five minutes, in PromQL:

```text
histogram_quantile(0.95, rate(kiva_request_duration_seconds_bucket[5m]))
```

A Prometheus configuration to collect them:

```yaml
scrape_configs:
  - job_name: 'kivadb'
    metrics_path: /metrics
    static_configs:
      - targets: ['localhost:8080']
```

Storage metrics describe the workspace of the user making the request (the `default` one for kiva). With authentication on, give Prometheus an account (`basic_auth`).

### 27.3 What to watch

| Signal | What it suggests |
| :--- | :--- |
| `kiva_checksum_errors_total` rises | The disk or the file is being damaged: run `check`, look at the hardware |
| `kiva_storage_fragmentation_ratio` stays high | Auto-compaction is off or cannot keep up |
| `kiva_replication_lag_ops` keeps growing | A replica is too slow or the link is poor |
| `kiva_failed_logins_total` rises fast | Someone is guessing passwords |
| `kiva_changes_consumer_lag` keeps growing | A program reading the change journal is stopped or too slow: past the retention its cursor expires |
| `kiva_memory_bytes` near your RAM | Too many keys for this machine |

---

## Chapter 28 — Using KivaDB inside a Rust program

Besides connecting to a running KivaDB ([24.5](#245-rust)), a Rust program can contain the engine itself: no server, no network, the data files are opened directly by your program. This is the **embedded** mode.

### 28.1 The storage engine alone

```rust
use kivadb::{Engine, EngineOptions, Value};

fn main() -> Result<(), kivadb::KivaError> {
    let db = Engine::open("data/store.kiva", EngineOptions::default())?;

    db.put("greeting", &Value::Str("hello".into()), 0, false)?;
    if let Some((value, entry)) = db.get("greeting")? {
        println!("{} ({})", value.display(), entry.ty.name());
    }

    for (key, _) in db.range(Some("a"), Some("m"), false, 10) {
        println!("{key}");
    }

    db.delete("greeting")?;
    db.close();
    Ok(())
}
```

* `put(key, &value, expires_at, msgpack)`: `expires_at` is a Unix time in milliseconds (0 = never; `kivadb::util::deadline_in(seconds)` computes one); `msgpack` chooses the encoding of documents.
* `Engine` can be shared between threads (`open` returns an `Arc`).
* Also available: `apply_batch` (atomic groups), `compact`, `check`, `create_index`, `query`, `stats`.

`EngineOptions` controls compression, auto-compaction and durability:

```rust
use kivadb::compress::Compression;

let options = EngineOptions {
    compression: Compression::Lz4,
    sync_writes: true,
    ..Default::default()
};
```

### 28.2 The full command language

`Instance` is a whole KivaDB (home directory, configuration, users, workspaces); `Session` is one user working in it.

```rust
use kivadb::{Instance, Options, Session};
use std::path::Path;

fn main() -> Result<(), kivadb::KivaError> {
    let instance = Instance::open(Path::new("./my-home"), Options::default())?;
    let mut session = Session::new(instance.clone(), true, None);

    let out = session.execute(r#"set user:1 {"name": "Alice"}"#);
    println!("{}", out.text());                 // OK: user:1 saved.
    println!("{}", session.execute("get user:1.name").text());

    instance.shutdown();
    Ok(())
}
```

`execute` returns the lines the command printed (`out.lines`, each flagged as error or not) and `out.has_err()` tells whether it failed.

Only one process can open a given data file or home directory at a time. If KivaDB is already running on that home, connect to it with the client instead.

---

## Chapter 29 — Deploying KivaDB on a server

This chapter is for the day KivaDB leaves your own computer: a Linux machine at a hosting provider, a Windows server, a Mac, or a Docker container. The database is the same everywhere; what changes is how the system starts it and keeps it running.

### 29.1 What every deployment has in common

Whatever the system, a hosted KivaDB needs five things:

1. **The executable** `kivadb` (downloaded or built for that system, see [Chapter 1](#chapter-1--installation)).
2. **A home directory** that survives restarts: configuration, users and data live there.
3. **A password** for the `kiva` administrator, and authentication turned on.
4. **A listening address** other machines can reach (`0.0.0.0` = every network interface).
5. **Open ports** in the firewall: 2911 for shells, Kiva Studio and client libraries; 8080 for the REST API and 6379 for Redis clients, only if you use them.

KivaDB refuses to listen on a non-local address until point 3 is done ([16.7](#167-opening-kivadb-to-the-network)), so an open, password-less database cannot be published by accident.

### 29.2 Configuring with environment variables

On a server, settings are usually given through the environment rather than typed in a shell. `kivadb serve` and `kivadb start` read these variables:

| Variable | Effect | Same as |
| :--- | :--- | :--- |
| `KIVADB_HOME` | Home directory | `--home` |
| `KIVADB_BIND` | Address to listen on | `--bind`, `network.bind` |
| `KIVADB_PORT` | KivaDB protocol port | `--port`, `network.port` |
| `KIVADB_HTTP_PORT` | REST API port | `--http-port`, `http.port` |
| `KIVADB_RESP_PORT` | Redis-compatible port | `--resp-port`, `resp.port` |
| `KIVADB_NO_UPDATE_CHECK` | `1` turns off the daily check for a new release ([1.6](#16-updating)) | |
| `KIVADB_RELEASES_URL` | Where `kivadb update` reads the latest release (a mirror); GitHub by default | |
| `KIVADB_ADMIN_PASSWORD` | Secures a new instance, see below | |

A command-line option wins over a variable, and a variable wins over `kiva.conf`.

**`KIVADB_ADMIN_PASSWORD`** does the two security steps for you when the server starts:

* if the `kiva` account still has an empty password, it receives this one;
* authentication is turned on.

It never overwrites a password that already exists: once you have changed the password with `modify user "kiva" password "..."`, the variable is ignored for the account. You can leave it in place or remove it.

```bash
KIVADB_ADMIN_PASSWORD='a-long-secret' KIVADB_BIND=0.0.0.0 kivadb serve
```

```text
  Security     : administrator password set, authentication turned on
KivaDB 1.7.1 server
  Home         : /var/lib/kivadb
  TCP protocol : 0.0.0.0:2911 (binary and plain text)
  TLS          : on every port, certificate SHA-256 77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6
  REST API     : http://0.0.0.0:8080  (docs: /docs, metrics: /metrics, events: /ws, /v1/stream)
  Redis (RESP) : 0.0.0.0:6379  (redis-cli and Redis client libraries)
  Auth         : required
  Replication  : master
Press Ctrl+C to stop.
```

### 29.3 Linux (systemd)

This is the usual case at a hosting provider. The steps below download KivaDB, create a dedicated system user, install the executable and let systemd start KivaDB at boot and restart it if it stops.

```bash
curl -LO https://github.com/fomadev/kiva-db/releases/download/v1.7.1/kivadb_linux_x64.tar.gz
tar -xzf kivadb_linux_x64.tar.gz
sudo useradd --system --home-dir /var/lib/kivadb --create-home kivadb
sudo install -m 755 kivadb /usr/local/bin/kivadb
```

Put the password in a file only root can read, `/etc/kivadb.env`:

```ini
KIVADB_HOME=/var/lib/kivadb
KIVADB_BIND=0.0.0.0
KIVADB_ADMIN_PASSWORD=a-long-secret
```

```bash
sudo chmod 600 /etc/kivadb.env
```

Create `/etc/systemd/system/kivadb.service`:

```ini
[Unit]
Description=KivaDB database
After=network-online.target
Wants=network-online.target

[Service]
User=kivadb
Group=kivadb
EnvironmentFile=/etc/kivadb.env
ExecStart=/usr/local/bin/kivadb serve
Restart=on-failure
RestartSec=2
# Give KivaDB time to flush and write its index snapshot.
TimeoutStopSec=30
KillSignal=SIGINT
NoNewPrivileges=true
ProtectSystem=full

[Install]
WantedBy=multi-user.target
```

Start it and check:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now kivadb
sudo systemctl status kivadb
sudo journalctl -u kivadb -f
```

Open the ports (example with `ufw`; open 8080 and 6379 only if you use the REST API or Redis clients):

```bash
sudo ufw allow 2911/tcp
sudo ufw allow 8080/tcp
sudo ufw allow 6379/tcp
```

To run commands on the server itself, use the same home directory as the service:

```bash
sudo -u kivadb KIVADB_HOME=/var/lib/kivadb kivadb -u kiva
```

`systemctl stop kivadb` sends an interrupt; KivaDB shuts down cleanly. Use `serve` (not `start`) under systemd: the service manager wants the process to stay in the foreground.

### 29.4 Windows

Copy `kivadb.exe` to a fixed place, for example `C:\KivaDB\kivadb.exe`, and choose a data folder, for example `C:\KivaDB\data`.

Set the variables for the whole machine (an administrator terminal), then open a new terminal:

```bash
setx /M KIVADB_HOME "C:\KivaDB\data"
setx /M KIVADB_BIND "0.0.0.0"
setx /M KIVADB_ADMIN_PASSWORD "a-long-secret"
```

Have Windows start KivaDB when the machine boots, before anyone logs in:

```bash
schtasks /Create /TN "KivaDB" /TR "\"C:\KivaDB\kivadb.exe\" serve" /SC ONSTART /RU SYSTEM /RL HIGHEST
schtasks /Run /TN "KivaDB"
```

Allow the ports through the firewall (the last two only if you use the REST API or Redis clients):

```bash
netsh advfirewall firewall add rule name="KivaDB" dir=in action=allow protocol=TCP localport=2911
netsh advfirewall firewall add rule name="KivaDB REST" dir=in action=allow protocol=TCP localport=8080
netsh advfirewall firewall add rule name="KivaDB Redis" dir=in action=allow protocol=TCP localport=6379
```

Check with `kivadb status`, stop with `kivadb stop -u kiva` (it asks for the password). Once the first start has set the password, you may delete the `KIVADB_ADMIN_PASSWORD` variable.

On your own PC, for development, none of this is needed: `kivadb start` is enough.

### 29.5 macOS

For development, `kivadb start` is enough. To have KivaDB start with the Mac, describe it to `launchd` in `~/Library/LaunchAgents/dev.foma.kivadb.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.foma.kivadb</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/kivadb</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/kivadb.log</string>
  <key>StandardErrorPath</key><string>/tmp/kivadb.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/dev.foma.kivadb.plist
```

Data stays in `~/.kivadb`. To accept connections from other machines, add an `EnvironmentVariables` dictionary with `KIVADB_BIND` and `KIVADB_ADMIN_PASSWORD`, as on Linux.

### 29.6 Docker

The official image is `fomadev0/kiva-db` on Docker Hub. It holds only the `kivadb` executable on a small Debian base, runs as an unprivileged user and keeps everything in `/data`.

| Tag | Meaning |
| :--- | :--- |
| `fomadev0/kiva-db:1.7.1` | Exactly version 1.7.1. Use this on servers: it never changes under you |
| `fomadev0/kiva-db:latest` | The most recent version |

**Run it:**

```bash
docker run -d --name kivadb -e KIVADB_ADMIN_PASSWORD=a-long-secret -p 2911:2911 -p 8080:8080 -p 6379:6379 -v kivadb-data:/data fomadev0/kiva-db:1.7.1
```

| Part | Meaning |
| :--- | :--- |
| `-d --name kivadb` | Run in the background, under the name `kivadb` |
| `-e KIVADB_ADMIN_PASSWORD=...` | Password of the `kiva` administrator, installed at the first start. **Required**: inside a container KivaDB listens on every interface, so it insists on authentication |
| `-p 2911:2911` | Publish the KivaDB protocol port |
| `-p 8080:8080` | Publish the REST API (leave out if unused) |
| `-p 6379:6379` | Publish the Redis-compatible port (leave out if unused) |
| `-v kivadb-data:/data` | Keep the data in a named volume, so it survives the container |

Every variable of [29.2](#292-configuring-with-environment-variables) can be passed with `-e`. Without the password the container stops at once and its log says why:

```text
Error: refusing to listen on 0.0.0.0 without authentication.
```

**Use it.** From Kiva Studio or any client: the address of the machine, port 2911, user `kiva`, your password. A shell inside the container:

```bash
docker exec -it kivadb kivadb -u kiva
```

Everyday commands:

```bash
docker logs -f kivadb
docker stop kivadb
docker start kivadb
```

`docker stop` lets KivaDB shut down cleanly. The image declares a health check (`kivadb status`), visible in `docker ps`.

**With Compose.** The repository's `docker-compose.yml` does the same as the `docker run` line above. Copy it to the server, then:

```bash
KIVADB_ADMIN_PASSWORD=a-long-secret docker compose up -d
```

On Windows PowerShell, set the variable first: `$env:KIVADB_ADMIN_PASSWORD = "a-long-secret"`, then `docker compose up -d`.

**Backup.** While the container runs, `kivadb backup` writes the archive inside the volume; copy it out with `docker cp`:

```bash
docker exec kivadb kivadb backup -u kiva -p "$KIVADB_ADMIN_PASSWORD" -o /data/backups/kivadb-$(date +%F).tar.gz
docker cp kivadb:/data/backups/. ./backups/
```

To restore, put the archive in an empty volume and run `kivadb restore` there before starting the container ([17.7](#177-backups)):

```bash
docker run --rm -v kivadb-new:/data -v "$PWD/backups":/backups fomadev0/kiva-db restore /backups/kivadb-2026-10-22.tar.gz --target /data
```

**Upgrade.** Download the new image, remove the container, run it again with the same volume. The data is untouched:

```bash
docker pull fomadev0/kiva-db:latest
docker rm -f kivadb
```

then the `docker run` line again with the new tag. With Compose: change the tag in the file, then `docker compose up -d`.

**Building the image yourself.** For an ARM server (the published image is for Intel and AMD processors) or a modified KivaDB, build from the source folder, then use `kivadb:local` instead of `fomadev0/kiva-db:1.7.1` in the commands above:

```bash
docker build -t kivadb:local .
```

### 29.7 Before exposing KivaDB to the Internet

| Check | Why |
| :--- | :--- |
| `kiva` has a long, unique password | It is the administrator of everything |
| One account per person or application, with the smallest role that works | A leaked `viewer` password cannot destroy data |
| Only the needed ports are open, ideally only to known addresses | Fewer doors |
| `tls.required` is `true` and clients connect with TLS | Otherwise the password and the data cross the Internet in clear ([16.11](#1611-encrypted-connections-tls)) |
| Data is encrypted at rest, the key is kept apart from the data and from the backups | A copied disk or backup then reveals nothing ([16.12](#1612-encryption-at-rest)) |
| Backups exist and were restored once as a test | A backup that was never restored is a hope, not a backup |
| `audit failed-logins` and the metrics are looked at | To notice password guessing early |

Another safe way, which needs no open port at all, is an SSH tunnel. On your own computer:

```bash
ssh -L 2911:localhost:2911 you@your-server
```

While that command runs, connect Kiva Studio to `localhost`, port 2911: the traffic travels encrypted inside SSH, and the server can keep listening on `127.0.0.1` only.

---

## Chapter 30 — Troubleshooting

Start with `kivadb doctor` ([17.9](#179-kivadb-doctor--a-health-check-of-the-home)): it checks the ports, the access, the certificate, the key and every record, and says what to do.

### 30.1 Starting

| What you see | Cause and remedy |
| :--- | :--- |
| `KivaDB is already running (pid ...)` | Nothing to do: it is started. `kivadb` opens a shell on it |
| `Error: cannot listen on 127.0.0.1:2911: ...` | Another program uses the port. Stop it, or choose another: `kivadb start --port 2912` (or `config set network.port 2912`) |
| `[WARN] REST API not started: cannot listen on ...:8080` | Port 8080 is taken. KivaDB works; only the REST API is missing. Use `--http-port` or `config set http.port` |
| `[WARN] Redis-compatible listener not started: cannot listen on ...:6379` | A real Redis probably uses port 6379. KivaDB works without the Redis listener. Use `--resp-port`, `config set resp.port`, or `--no-resp` |
| `kivadb: command not found` or `is not recognized as ... command` | The folder of `kivadb` is not in the `PATH`, or the terminal was opened before you changed it. See [1.4](#14-make-it-available-everywhere) |
| macOS: "kivadb cannot be opened because the developer cannot be verified" | Run `xattr -d com.apple.quarantine kivadb` once, see [1.1](#11-download-the-ready-made-executable) |
| Docker: the container stops right away | `docker logs kivadb` gives the reason; usually `KIVADB_ADMIN_PASSWORD` is missing ([29.6](#296-docker)) |
| `Error: refusing to listen on ... without authentication.` | You set a non-local `bind` without securing KivaDB. See [16.7](#167-opening-kivadb-to-the-network) |
| `Error: database is locked by another process` | Another process has these files open. Check `kivadb status` |
| `ERR_DATA_CORRUPTED: ...store.kiva.meta is damaged` | The file of index and schema definitions cannot be read (a disk error, or a copy cut short). KivaDB does not open the workspace without its unique indexes and schemas, which would no longer be enforced. Restore the file from a backup ([17.7](#177-backups)); or move it away, open the workspace, and create its indexes and schemas again. `kivadb doctor` reports it too |
| `Error: invalid database file: missing KIVA signature` | The file is not a KivaDB data file. KivaDB does not touch it |
| `Error: invalid database file: unsupported storage format N` | The file was written by a newer KivaDB (format 2: by 2.0, with dates in it). Use that version, or restore the backup made before upgrading |
| `kivadb start` says the server stopped right away | The reason is in the last lines of `server.log`, which are displayed |
| `kivadb update`: `permission denied` | The executable is in a protected folder: `sudo kivadb update` (Linux, macOS) or a terminal run as administrator (Windows) |
| `kivadb update`: `GitHub limits anonymous requests` | Too many requests from your network to GitHub in the last hour. Try again at the time shown |
| `kivadb update`: `cannot reach GitHub` | No Internet connection, or a proxy or firewall blocks api.github.com |

### 30.2 Commands

| Message | Meaning |
| :--- | :--- |
| `Key 'x' already exists. Use 'update' to change it.` | `set` only creates |
| `Key 'x' not found. Use 'set' to create it.` | `update` only modifies |
| `String values like 'x' must be quoted` | Put text between quotes |
| `Numbers and Booleans must not be quoted.` | You gave a type prefix `number` or `boolean` with a quoted value |
| `Cannot update [number] with a value formatted as [string].` | `update` keeps the type; use `change` to migrate |
| `InvalidKeyName: '44' cannot be purely numeric.` | Add a letter: `id44` |
| `'get' is a reserved keyword and cannot be used as a key.` | Use backticks: `` `get` `` |
| `Syntax Error: Unclosed quote detected (").` | A quote is missing |
| `Invalid JSON literal: ...` | The array or object is not valid JSON (field names need double quotes) |
| `Unknown command: 'x'. Try 'help'.` | Typing mistake |
| `ERR_TRANSACTION_CONFLICT: ...` | Someone changed the same key; redo the transaction |
| `ERR_DATA_CORRUPTED: ...` | The record is damaged on disk; see [17.4](#174-integrity-check) |
| `READONLY: this node is a replica` | Write on the master |

### 30.3 Access

| Message | Meaning |
| :--- | :--- |
| `permission denied: authentication required. Use: auth <user> <password>` | Log in first |
| `permission denied: invalid username or password` | Wrong credentials |
| `permission denied: this command requires the admin role` | Ask an administrator |
| `permission denied: you are in no workspace` | The account landed in none: `workspace list`, then `workspace switch <name>`; or log in as `<user>@<workspace>` ([16.5](#165-logging-in)) |
| `permission denied: this command requires the admin role, or write access to every workspace of project 'p'` | `compact` and `workspace create` in a project need write on `p.*`, or an administrator |
| `NOPERM no access to workspace '...'` | The account has no grant there, or the workspace does not exist: `grant ... on <workspace> to "<user>"` |
| `permission denied: the viewer role is read-only` | Your role cannot write |
| `permission denied: you only have read access to workspace 'x'` | You were granted *read* only |
| `permission denied: no access to workspace 'x'` | An admin must `grant` you access |
| `permission denied: accounts with an empty password can only log in from the local shell` | Give the account a password |
| `this server requires an encrypted connection (TLS) from other machines` | Connect with TLS: `--tls`, the Kiva Studio switch, `tls` in the driver, `rediss://` ([16.11](#1611-encrypted-connections-tls)) |
| `the server's certificate (...) is not the one expected` | The server's certificate changed. If it was renewed (`kivadb tls renew`), remove its line from `known_servers` and trust the new one; otherwise someone may be intercepting the connection |

### 30.4 Other situations

**"I changed a key in one terminal and the other does not see it."** Both terminals must use the same home directory. `kivadb status` and `workspace current` show where each one is. Also check that you are not inside an uncommitted transaction.

**"My data is not where I expect."** Data lives in the home directory ([1.5](#15-where-kivadb-keeps-its-files)), not in the folder you launched `kivadb` from. `stats` shows the exact path on its last line.

**"The data file keeps growing."** It is fragmentation; see `stats`. Run `compact`, and check `autocompact status`.

**"KivaDB uses a lot of memory."** All keys are kept in RAM. Shorter keys, fewer keys, or documents grouping several small values reduce it.

**"A key disappeared."** It may have had a TTL. `scan` shows remaining times for keys that expire.

**"I forgot the kiva password."** See [16.10](#1610-emergency-access).

---

## Chapter 31 — Command cheat sheet

### Command line

| Command | Effect |
| :--- | :--- |
| `kivadb start` | Start the database in the background |
| `kivadb stop` | Stop it (`kivadb stop -u kiva` when authentication is on) |
| `kivadb status` | Is it running? |
| `kivadb` | Open a shell |
| `kivadb serve` | Run the server in the foreground |
| `kivadb -e "<command>"` | Run a command and exit (repeatable) |
| `kivadb run <file.kva> [--keep-going]` | Run a command file ([3.10](#310-command-files-kva)) |
| `kivadb -w <workspace>` | Open a workspace |
| `kivadb -u <user> [-p <password>]` | Log in |
| `kivadb --connect <host:port> [--tls]` | Shell on a remote KivaDB, encrypted with `--tls` (`--tls-fingerprint <fp>` in scripts) |
| `kivadb tls` / `kivadb tls renew` | Show the TLS certificate and its fingerprint / create a new one |
| `kivadb keygen` | Print a new encryption key ([16.12](#1612-encryption-at-rest)) |
| `kivadb doctor [--json]` | Look for what could stop KivaDB or lose data, reading only ([17.9](#179-kivadb-doctor--a-health-check-of-the-home)) |
| `kivadb backup -o <file.tar.gz> [-w <workspace>]` | Save the home or a workspace, even while KivaDB runs ([17.7](#177-backups)) |
| `kivadb restore <file.tar.gz> [--target <dir>] [--as <name>] [--check]` | Check, then restore an archive (KivaDB stopped) |
| `kivadb --no-listen` | Shell on local files that other shells cannot join |
| `kivadb --home <folder>` | Use another home directory |
| `kivadb --skip-auth` | Start without authentication (maintenance) |
| `kivadb --lua-debug` | Trace Lua scripts |
| `kivadb start --resp-port <n>` / `--no-resp` | Redis-compatible port, or none |
| `kivadb start --port <n>` / `--http-port <n>` / `--no-http` / `--bind <address>` | Ports and listening address |
| `kivadb update` / `kivadb update --check` | Install the latest release / only tell whether there is one |
| `kivadb --version`, `--help` | Version, options |

### Data

| Command | Effect |
| :--- | :--- |
| `set [type] <key> <value> [ttl <sec>] [if not exists]` | Create (`if not exists`: skip if present) |
| `set datetime <key> "2026-10-27 14:30"`, `set date <key> "..."`, `set time <key> "..."` | A moment ([5.8](#58-date-datetime-time)) |
| `set <key> now()` / `today()` / `current_time`, `+ 7 days`, `- 2 hours` | The moment of the action, moved by durations |
| `add <prefix> <value> [ttl <sec>]` | Create a key named by KivaDB: `<prefix>` + a new identifier ([6.7](#67-add--let-kivadb-name-the-key)) |
| `get [type] <key>` | Read (`get user:1.name` for a field) |
| `update [type] <key> <value>` | Replace, same type |
| `del [type] <key> [if exists]` | Delete |
| `del all keys` | Empty the workspace (admin) |
| `has [type] <key>` | Does it exist? |
| `typeof <key>` | Its type |
| `change [type] <old> to [type] <new> [value]` | Rename or migrate |
| `bump [type] <key> add\|set <sec>` | Extend or reset the TTL |
| `print <expr>, <expr>` | Calculate and display; `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()`, `weekday()`, `now()`, `today()` |
| `timezone [<name> \| default]` | Show or change the time zone of the session ([5.9](#59-time-zones)) |

### Listing and querying

| Command | Effect |
| :--- | :--- |
| `keys [like <pat>] [from <A> to <B>] [limit N] [reverse]` | Key names |
| `scan [pattern] [limit N]` | Keys with details |
| `range <A> to <B> [reverse] [limit N] [return f1, f2]` | Keys and values |
| `json get <key> [<.path>]` | Read a field |
| `json set <key> <.path> <value>` | Change a field |
| `json delete <key> <.path>` | Remove a field |
| `json append <key> <.path> <value> [, ...]` | Add at the end of an array |
| `json insert <key> <.path[n]> <value>` | Insert into an array |
| `json incr <key> <.path> <number>` | Add to a number |
| `json type / len / keys <key> [<.path>]` | Type, length, field names |
| `get where <pat>.<field> <op> <value> [and\|or ...] [order by <field> [desc]] [limit N] [after <key>] [return f1, f2]` | Find documents; `and`, `or`, `not`, parentheses; sorted, a page after a key |
| `<field> [not] in [v1, v2...]`, `<field> [not] like\|ilike "Kin%"` | One of several values; a text pattern (`ilike`: capitals or not) ([11.1](#111-the-condition)) |
| `count` / `sum` / `avg` / `min` / `max` `where ...` or `<pat>[.<field>] [where ...]` `[group by <field>]` | Aggregates |
| `create index on <pat> (<field>) [using btree\|hash]` | Create an index; also `on <pat>.<field>` (admin) |
| `create unique index on <pat> (<f1>[, <f2>...])` | Refuse a second document with the same values ([11.7](#117-unique-indexes)) (admin) |
| `drop index on <pat> (<field>[, ...])` | Remove it (admin) |
| `indexes` | List indexes |
| `schema set <pat> {"<field>": {"type": ..., "required": true, "default": ...}}` | Rules and defaults for the documents of a pattern ([11.8](#118-schemas-rules-for-the-documents-of-a-pattern)) (admin) |
| `schemas`, `schema show / check / drop <pat>` | List, show, find the documents that do not follow it, remove it (`drop`: admin) |
| `explain <query>` | Show the execution plan, reading no document |
| `explain analyze <query>` | Run a read query and show what it read, filtered, sorted and returned, and how long each stage took |
| `get similar <pat>.<field> to <vector> [where <field> <op> <value> [and ...]] [limit N] [metric m] [ef N] [exact] [return f1, f2]` | Vector similarity search, filtered on other fields ([12.9](#129-filtering-on-other-fields)) |
| `create vector index on <pat> (<field>)` | Keep vectors in memory, exact search (admin) |
| `create vector index on <pat> (<field>) using hnsw [metric m] [m N] [ef N] [ef_search N] [quantize sq8\|none]` | Approximate index for large collections (admin) |
| `tune vector index on <pat> (<field>) [recall 0.95] [limit N] [sample N] [queries "<pat>"] [auto]` | Measure an HNSW index and keep the `ef_search` it needs; `auto`: again as it grows ([12.12](#1212-tuning-an-hnsw-index)) (admin) |
| `tune vector index on <pat> (<field>) auto off` / `ef_search N\|default` | Stop tuning it again / set `ef_search` by hand (admin) |
| `semantic lookup / store / stats / clear` | Semantic cache of answers ([12.8](#128-semantic-cache-reuse-answers-to-the-same-question)) |
| `drop vector index on <pat> (<field>)` | Remove it (admin) |
| `convert <key> to msgpack\|json` | Change a document's encoding |

### Transactions

| Command | Effect |
| :--- | :--- |
| `begin` | Start |
| `commit` | Apply everything |
| `rollback` | Cancel everything |
| `savepoint <name>` | Place a bookmark |
| `rollback to <name>` | Return to a bookmark |
| `transaction status` | Show the open transaction |

### Maintenance

| Command | Effect |
| :--- | :--- |
| `stats` | Dashboard |
| `memory` | Where the workspace's memory goes, and its bound |
| `memory pin "<pattern>"` / `memory unpin "<pattern>"` | Keep the values of matching keys in memory (admin) |
| `compact` | Reclaim disk space (admin, or a manager of the workspace's project) |
| `autocompact on\|off\|status\|threshold <N>` | Automatic compaction |
| `check` / `check repair` | Verify / repair |
| `hint info` | State of the index snapshot |
| `encryption` | Encryption at rest: key in use, records still in clear (admin) |
| `backup to "<file.tar.gz>" [workspace <name>]` | Archive the home or a workspace, written by the server (admin) |
| `compress lz4\|zlib\|none` | Re-encode the workspace (admin) |
| `bench write <N>` / `bench read <N>` | Measure speed (admin) |
| `shutdown` | Stop the database (admin) |

### Workspaces and configuration

| Command | Effect |
| :--- | :--- |
| `workspace list [<project>]` / `workspace current` | Show the workspaces you can reach |
| `workspace switch <name>` | Change workspace |
| `workspace create <name> [path]` | Create (admin; in a project, also its managers, without a path) |
| `workspace move <name> to <new name>` | Rename, or move into or out of a project (admin) |
| `workspace remove <name>` | Unregister (admin) |
| `project list` | Show the projects and their workspaces |
| `project create <name>` / `project remove <name>` | Create / remove an empty project (admin) |
| `config list` | All settings (admin) |
| `config get <key>` / `config set <key> <value>` | Read / change a setting (admin) |

### Users

| Command | Effect |
| :--- | :--- |
| `whoami` | Who am I, where |
| `auth <user>[@<workspace>] <password>` | Log in, optionally straight into a workspace |
| `users` | List accounts (admin) |
| `create user "<n>" identified by "<p>" [role <r>] [grant read\|write on <scopes>] [default workspace <w>]` | Create an account, with its access (admin) |
| `modify user "<n>" password "<p>"` | Change a password |
| `modify user "<n>" role "<r>"` | Change a role (admin) |
| `modify user "<n>" default workspace <w>\|none` | Where the user lands at login (admin) |
| `modify user "<n>" timezone <zone>\|none` | The user's time zone (their own, or admin) |
| `drop user "<n>"` | Delete an account (admin) |
| `grant read\|write on <scope>[, <scope>...] to "<n>"` | Give access: `travel.places`, `travel.*`, `*.places`, `*.*` (admin) |
| `grant admin to "<n>"` | Make an admin (admin) |
| `revoke read\|write\|admin [on <scope>[, <scope>...]] from "<n>"` | Remove access (admin) |
| `audit last <N>` / `audit since "<date>"` / `audit failed-logins` | Read the audit log (admin) |

### Scripting and events

| Command | Effect |
| :--- | :--- |
| `eval '<lua>' [args]` | Run Lua code |
| `script load "<file>" as "<n>"` | Store a script (admin) |
| `script run "<n>" [args]` | Run a stored script |
| `script list` / `script show "<n>"` / `script delete "<n>"` | Manage scripts |
| `create trigger [name] on set\|update\|del "<pat>" run "<script>"` | Create a trigger (admin) |
| `drop trigger <name>` / `triggers` | Remove / list triggers |
| `bench script "<n>" iterations <N> [args]` | Time a script (admin) |
| `subscribe <pattern>` / `unsubscribe [pattern]` | Change notifications |
| `changes enable [keys only] [retain 24h] [max 256mb]` / `changes disable` | Keep a durable change journal, or remove it ([22.5](#225-the-change-journal-changes-kept-read-again-from-a-cursor)) (admin) |
| `changes read [from beginning|now|<cursor>] [as <consumer>] [limit N] [like <pat>]` | The changes after a cursor |
| `changes status` / `changes forget <consumer>` | Journal and consumers / forget one (admin) |

### Replication (admin)

| Command | Effect |
| :--- | :--- |
| `replica status` | State of replication |
| `replica of <host> <port>` / `replica of none` | Follow a master / become a master |
| `replica start` / `stop` / `sync` / `promote` | Control the link |
| `wait <N> [timeout_ms]` | Wait for N replicas |
| `failover to <replica>` | Hand over the master role |

### Shell

| Command | Effect |
| :--- | :--- |
| `help`, `h` | List of commands |
| `source "<file.kva>"` | Run a command file |
| `clear` | Clear the screen |
| `ping` | Answers `PONG` |
| `version` | Version of KivaDB |
| `exit`, `quit` | Leave the shell |

---

## Chapter 32 — Limits and scope

KivaDB 1.7.1 keeps the whole database on each node, with replicas and automatic failover. Knowing what it does not do is as useful as knowing what it does.

**Not included in 1.7.1**

* **Clustering.** No sharding of data across machines, no quorum reads and writes, no distributed queries: every node holds the whole database. A majority of nodes is used for one thing only, to elect a new master ([25.8](#258-automatic-failover)); replication stays asynchronous by default.
* **SQL and the PostgreSQL protocol.** The query language is the one described in this manual. Tools that require PostgreSQL (Prisma, DBeaver's SQL editor) cannot connect.
* **Vector indexes on disk, and dedicated vector engine features.** Vector indexes, exact or HNSW, live in memory, bounded by `vector.max_ram_mb`: the vectors must fit in the RAM of one machine. 8-bit quantization only (no product quantization), no graph searched from disk, no hybrid (keyword and vector) search, filters joined by `and` only, embeddings computed by the application ([12.10, 12.11](#1210-memory-bounds)).
* **The full Redis command set**: sharded publish/subscribe, Redis functions, cluster commands and modules are missing (refused with a clear error); sets, sorted sets, lists and streams are meant for up to tens of thousands of members per key, not millions, since each command rewrites the whole value (bounded by `resp.max_collection_members`); blocked clients are woken together rather than in order ([23.3](#233-supported-commands)).
* **A system service installer.** See [section 2.6](#26-starting-kivadb-with-the-computer).
* **Ready-made executables for every system.** Releases contain Windows x64, Linux x64 and macOS Apple Silicon executables, and a Docker image for Intel and AMD processors. Other systems (Linux on ARM, Intel Macs) build from the source ([1.3](#13-build-from-the-source-code)).

**Design limits**

* All keys must fit in memory ([19.6](#196-limits)).
* Replication copies whole nodes: every workspace is on every replica. A workspace is never split across machines (no sharding): its data must fit on one node, its keys in that node's memory ([19.6](#196-limits)).
* Conditions have no regular expressions: `like` and `ilike` cover prefixes, suffixes and words inside a text ([11.1](#111-the-condition)).
* A transaction lives in one connection; it cannot span several HTTP requests.
* Listing and query commands do not show the uncommitted writes of your own transaction.
* Strings have no escape sequences: a text cannot contain both kinds of quotes at once from the shell (it can through the REST API or the binary protocol).
* Changes to grants reach a connected user at their next login or workspace switch.

What is planned next, and what was considered and set aside with the reasons, is in the [roadmap](ROADMAP.md).

---

## Chapter 33 — Upgrading from KivaDB 1.x to 2.0

KivaDB 2.0 opens the homes of 1.x as they are: there is no conversion to run and no command to learn. What changes is the access model of accounts, which affects scripts that create users, and the replication between versions. This chapter goes through it in the order of an upgrade. The full list of changes is in the [changelog](CHANGELOG.md).

### 33.1 What changes, what stays

| | In 2.0 |
| :--- | :--- |
| Data files, compression, encryption at rest | Read as they are. Nothing is rewritten at the upgrade |
| Indexes (B-tree, unique, vector), HNSW graphs, hint files | Read as they are |
| Configuration (`kiva.conf`), TLS certificate, audit log | Kept |
| Accounts | Migrated at the first start: each personal workspace `u_<name>` becomes a write grant and the account's default workspace ([16.4](#164-workspaces-and-grants)) |
| New accounts | Reach **no workspace** until an admin grants them some ([16.3](#163-managing-accounts)) |
| `compact` by an editor | Needs write access to every workspace of the project (`grant write on travel.*`), instead of the personal workspace |
| Storage format | Stays `1`, until the first `date`, `datetime` or `time` is written to a workspace: that file moves to format `2`, which 1.x refuses ([19.4](#194-the-data-file-byte-by-byte)) |
| Replication | A 2.x node and a 1.x node never follow each other ([33.5](#335-a-replication-group)) |

These homes are kept in the repository and opened by every test run of 2.0 (`tests/compat.rs`): plain, compressed, encrypted, with indexes, a unique index, expiry, an HNSW graph, workspaces and accounts, all written by the 1.7.1 executable.

### 33.2 Before you start

1. **Read what your applications rely on** in [33.6](#336-what-applications-may-have-to-change). Most need no change; scripts that create accounts do.
2. **Make a backup**, KivaDB running or not ([17.7](#177-backups)):

   ```bash
   kivadb backup --output /backups/kivadb-before-2.0.tar.gz
   ```

   It is the only way back once a workspace holds a date ([33.7](#337-going-back-to-1x)).
3. **Check the home** with `kivadb doctor` after the upgrade ([17.9](#179-kivadb-doctor--a-health-check-of-the-home)); 1.7.1 has no `doctor`, but `check` verifies every record ([17.4](#174-integrity-check)). Fix what it reports first: an upgrade is not the moment to discover a full disk.

### 33.3 Upgrading one server

1. Stop KivaDB: `kivadb stop` (or stop the service, [Chapter 29](#chapter-29--deploying-kivadb-on-a-server)).
2. Replace the executable with the one of 2.0 ([1.1](#11-download-the-ready-made-executable)). `kivadb update` does it too, but it installs the latest release, whatever it is: run it when you mean to move to 2.0, not out of habit.
3. Start KivaDB. At this first start, the accounts are migrated; nothing else is written.
4. Look at the accounts:

   ```text
   kiva> users
    USER                 | ROLE    | CREATED             | GRANTS
   ------------------------------------------------------------------------------
    app                  | editor  | 2026-10-12 09:44:55 | u_app=write (default: u_app)
    kiva                 | admin   | 2026-10-12 09:44:53 | all (admin)
   ```

   Each account that had a personal workspace now holds a grant on it and lands in it at login, as before. An account whose personal workspace no longer existed (removed with `workspace remove`) gets nothing: give it what it needs with `grant`.
5. Run `kivadb doctor`, then let your applications connect.

To give the personal workspaces clearer names, move them into a project afterwards, applications stopped: `workspace move u_app to travel.places` (admin, once `project create travel` is done) ([14.7](#147-renaming-or-moving-a-workspace)). Their grants and the default workspaces follow; applications that named `u_app` must then use the new name.

**With Docker**, change the tag of the image (`fomadev0/kiva-db:1.7.1` to the 2.0 tag) and start a new container on the same volume ([29.6](#296-docker)). The volume holds the home; the image holds only the executable.

### 33.4 Checking that it went well

* `kivadb doctor`: every line `OK`, or a reason you know.
* `check` in each workspace: `CORRUPTED: 0`.
* `indexes`: the same indexes as before, with the same counts.
* Each application: one read and one write with its own account.

### 33.5 A replication group

A node of 2.x and a node of 1.x **never follow each other**, in either direction, and say so in `replica status`:

* a 1.7.1 replica of a 2.0 master: `this master runs KivaDB 2.0.0, and a replica of KivaDB 1.x cannot follow it: upgrade the replica to KivaDB 2`;
* a 2.0 replica of a 1.7.1 master: `the master runs KivaDB 1.x, and a replica of KivaDB 2.0.0 cannot follow it: upgrade the master to KivaDB 2 first`.

The refusal comes before the snapshot, so the replica keeps its data. Without it, a 1.x replica stopped at the first date it received, after emptying its data for the snapshot; and a 2.0 replica received accounts of 1.x that it never migrated, so that they reached nothing on it, nor on it after a failover. `tests/compat.rs` checks both directions against the real 1.7.1 executable.

So a group is upgraded as a whole, in one maintenance window, during which it does not answer.

1. On every node, turn automatic failover off, so that a stopped master is not replaced: `config set failover.auto false`.
2. Back up the master (`kivadb backup`).
3. Stop the replicas, then the master.
4. Upgrade the master ([33.3](#333-upgrading-one-server)), start it and check it ([33.4](#334-checking-that-it-went-well)).
5. Upgrade and start each replica. It follows the master again by itself (the choice is in its configuration) and copies it whole: a full snapshot, then the stream. `replica status` shows `Link: connected`.
6. Turn automatic failover back on, on every node.


### 33.6 What applications may have to change

| If your application... | In 2.0 |
| :--- | :--- |
| Creates accounts, then writes with them | Give access in the same command: `create user "app" identified by "..." grant write on travel.* default workspace travel.places`. A new account otherwise reaches nothing |
| Logs in with an account of 1.x | Nothing: it lands in its personal workspace, as before |
| Reads the `users` structured request | The field `home` is now `default_workspace`, and may be `null` |
| Reads `whoami` | `workspace` and `access` are `null` for a session in no workspace; `can_compact` is new |
| Lets editors run `compact` | They need `grant write on <project>.*`; outside a project, only an admin compacts |
| Stores dates as text (`"2026-10-27"`) | Nothing: a text stays a text, and conditions already compare ISO dates as moments. Use the new types for new data if you want them checked and shown in a time zone ([5.8](#58-date-datetime-time)) |
| Shows the creation dates of accounts | They are in the server's time zone (`time.zone`), not always the machine's |
| Uses the Node.js driver | From 1.2.0, a JavaScript `Date` is stored as a `datetime` and comes back as a `Date` ([24.3](#243-nodejs)) |
| Uses Kiva Studio | A version that knows 2.0 shows projects, grants on scopes, default workspaces and the new types; an older one connects, but shows the personal workspace model of 1.x ([24.7](#247-kiva-studio-the-graphical-client)) |

Everything else (the command language of 1.x, the REST API, the binary protocol, the Redis commands) answers as before.

### 33.7 Going back to 1.x

Going back is possible for a workspace that holds no `date`, `datetime` or `time`: its file is still at format 1, and 1.7.1 reads it. A workspace that received one is at format 2: 1.7.1 refuses it (`unsupported storage format 2`) and leaves it untouched, rather than cutting those records off. For those, restore the backup made before the upgrade: the writes made since are lost.

The accounts saved by 2.0 carry the new model. 1.7.1 reads them, but gives each user back a `u_<name>` workspace reached without a grant, and ignores the grants on scopes such as `travel.*`. Check `users` after going back.

---

## Glossary

| Term | Meaning |
| :--- | :--- |
| **Admin, editor, viewer** | The three roles: everything / read and write / read only ([16.2](#162-roles)) |
| **Append-only log** | A file to which records are only ever added at the end |
| **Atomic** | Happens completely or not at all |
| **Audit log** | The journal of sensitive actions: logins, account changes, deletions |
| **Authentication** | Proving who you are with a user name and a password |
| **Binary protocol** | KivaDB's own network language on port 2911, made of frames ([20.2](#202-the-binary-protocol)) |
| **Checksum (CRC32)** | A number computed from data, stored with it, and recomputed at read time to detect damage |
| **Client** | Any program that connects to a running KivaDB: the shell, Kiva Studio, your application |
| **Commit** | Apply the changes of a transaction |
| **Compaction** | Rewriting the data file without obsolete records |
| **Cosine similarity** | A score between -1 and 1 telling how close two vectors point; 1 means the same direction |
| **Document** | A value with named fields (object) or a list (array) |
| **Driver** | A library that lets a programming language talk to KivaDB |
| **Embedding** | A vector produced by an AI model to represent the meaning of a text, an image or a sound |
| **Encryption at rest** | Data stored encrypted on disk, readable only with the key ([16.12](#1612-encryption-at-rest)) |
| **Epoch** | The number of the current master's term in a replication group; it grows at each failover ([25.8](#258-automatic-failover)) |
| **Failover** | Handing the master role to a replica, by command or automatically ([25.7](#257-failover)) |
| **Fragmentation** | The share of the data file occupied by obsolete records |
| **Grant** | Permission given to a user on a workspace |
| **Heartbeat** | A short message the master sends regularly so that replicas know it is alive |
| **Hint file** | A saved copy of the index, for fast start |
| **HNSW** | A graph of vectors that finds the nearest ones while comparing only a few ([12.5](#125-approximate-index-hnsw-for-large-collections)) |
| **Home directory** | The folder holding KivaDB's configuration, users and data |
| **Index** | The in-memory table telling where each key's value is on disk |
| **JSON** | The text format of documents: `{"name": "Alice"}` |
| **Key** | The name under which a value is stored |
| **Loopback (127.0.0.1, localhost)** | The address of the computer itself; only programs on the same machine can reach it |
| **Master / replica** | The node that receives writes / a node that copies it |
| **MessagePack** | A compact binary encoding of JSON-like data |
| **Path** | The address of a field inside a document: `.address.city` |
| **Pattern** | A key template with `*` and `?` |
| **Port** | The number that tells which program of a machine a network connection is for: 2911, 8080, 6379 |
| **Projection** | Keeping only some fields of the documents returned |
| **Quorum** | The number of nodes that must agree before a new master is elected; by default a majority |
| **REST API** | Access to KivaDB over plain HTTP, on port 8080 ([Chapter 21](#chapter-21--the-rest-api)) |
| **RESP** | The network protocol of Redis, spoken by KivaDB on port 6379 ([Chapter 23](#chapter-23--redis-compatibility)) |
| **Role** | What a user is allowed to do: admin, editor, viewer |
| **Rollback** | Cancel the changes of a transaction |
| **Savepoint** | A bookmark inside a transaction |
| **Secondary index** | An index on a field of documents, to speed up queries |
| **Semantic cache** | Answers kept for questions and given back for questions that mean the same ([12.8](#128-semantic-cache-reuse-answers-to-the-same-question)) |
| **Server-Sent Events (SSE)** | An HTTP connection that stays open to receive notifications ([22.4](#224-over-server-sent-events)) |
| **Shell** | The interactive command line opened by `kivadb`, with the `kiva>` prompt |
| **Sorted set** | A Redis set whose members carry a score and stay ordered by it ([23.8](#238-sets-and-sorted-sets)) |
| **Tombstone** | The record that marks a key as deleted |
| **Transaction** | A group of changes applied together |
| **Trigger** | A script run automatically when keys change |
| **TTL** | Time to live: the delay after which a key expires |
| **Value cache** | Optional copies of recently read values kept in memory ([Chapter 18](#chapter-18--keeping-values-in-memory-value-cache)) |
| **Vector** | A list of numbers, such as an embedding; KivaDB finds the closest ones ([Chapter 12](#chapter-12--vector-search-for-ai)) |
| **Vector index** | The vectors of a field kept in memory to make similarity search fast |
| **WebSocket** | A two-way connection opened from a web page, used for notifications ([22.3](#223-over-websocket)) |
| **Workspace** | An independent database inside one KivaDB |
