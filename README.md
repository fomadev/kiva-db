<p align="center">
  <img src="ban.png" alt="KivaDB Logo">
</p>

# KivaDB

KivaDB is a typed key-value and document database written in Rust. It stores data in an append-only, checksummed log, keeps an ordered index in memory, and ships as a single executable that works as an interactive shell, an embedded engine, or a network server with a REST API.

**Current release: 1.0.0**

## Highlights

* **Seven native types**: `string`, `number`, `boolean`, `null`, `array`, `object`, `bytes`, with strict type checking on update.
* **Durable storage**: every record carries a CRC32; torn writes and bit rot are detected, never served. Recovery after a crash or `kill -9` is automatic.
* **Instant startup**: a hint file (index snapshot) avoids rescanning the data file; only what was written after the snapshot is replayed.
* **Ordered keys**: range queries, pagination, reverse scans.
* **Documents**: JSON paths (`get user:1.address.city`), partial updates, secondary indexes, `where` filters, `count` / `sum`, projections, `explain`.
* **Transactions**: `begin` / `commit` / `rollback`, savepoints, optimistic conflict detection, atomic on disk.
* **Compression**: LZ4 or Zlib per record, transparent on read. MessagePack as an alternative document encoding.
* **Background maintenance**: auto-compaction that does not block readers or writers, TTL expiry, periodic snapshots.
* **Multi-user**: accounts with salted PBKDF2 password hashes, roles, per-workspace grants, audit log.
* **Workspaces**: several isolated databases behind one configuration file.
* **Server**: TCP (binary protocol and plain text for telnet), REST API with Basic auth, CORS and rate limiting, WebSocket change notifications, Prometheus metrics, built-in API documentation page.
* **Replication**: master to replicas, asynchronous or synchronous, `wait`, manual failover.
* **Lua scripting**: sandboxed, time-limited, atomic scripts; stored scripts; triggers.

## Build

KivaDB needs a stable [Rust](https://rustup.rs) toolchain and a C compiler (used once, to build the embedded Lua).

```bash
cargo build --release
```

The executable is `target/release/kivadb` (`kivadb.exe` on Windows). Run the test suite with `cargo test`.

## Quick start

```text
$ kivadb
KivaDB Shell v1.0.0
Type 'help' or 'h' for command list
kiva> set name "Fordi" and age 30
OK: name saved.
OK: age saved.
kiva> set user:1 {"name": "Alice", "role": "admin", "address": {"city": "Kinshasa"}}
OK: user:1 saved.
kiva> get user:1.address.city
user:1.address.city: Kinshasa [string]
kiva> print "Next year ${name} will be ${age + 1}"
Next year Fordi will be 31
kiva> count where user:*.role = "admin"
Count: 1
```

Data lives in the KivaDB home directory: `%APPDATA%\KivaDB` on Windows, `~/.kivadb` elsewhere (override with `--home <dir>` or the `KIVADB_HOME` environment variable). Wherever you start `kivadb`, you find the same data.

**Several terminals.** The first `kivadb` process opens the files; any other `kivadb` started on the same home attaches to it automatically. Every terminal sees every change immediately.

```text
kivadb                          interactive shell
kivadb -e "get name"            run a command and exit (repeatable)
kivadb -w projet_a              open a workspace
kivadb -u fordi                 log in (password is prompted)
kivadb --connect host:2911      shell on a remote server
kivadb serve                    TCP server + REST API
kivadb --help                   every option
```

## Syntax rules

| Element | Rule | Example |
| :--- | :--- | :--- |
| String | must be quoted, `"..."` or `'...'` (no escape sequences: use the other quote to embed one) | `"Hello"` |
| Number, boolean | must not be quoted | `42`, `-3.14`, `1e9`, `true` |
| Array, object | JSON literal | `["red", "green"]`, `{"name": "Fordi"}` |
| Null | the bare word | `null` |
| Bytes | hex, base64, or a quoted string with the `bytes` type | `0xDEADBEEF`, `b64:SGVsbG8=` |
| Key | bare text; not purely numeric; no spaces, quotes, brackets, `*` or `?`. Backticks allow reserved words | `user:1`, `` `set` `` |
| Chaining | `and` repeats a command | `get a and b` |
| Type prefix | optional, enforces the type | `set number age 30` |

A key holding `null` exists but has no type yet: `update` may give it a value of any type. Otherwise `update` never changes the type of a key; use `change` for that.

## Command reference

### Data

| Command | Description |
| :--- | :--- |
| `set [type] <key> <value> [ttl <sec>]` | Creates a key. Fails if it exists. |
| `update [type] <key> <value>` | Replaces the value of an existing key, keeping its type and TTL. |
| `get [type] <key>` | Reads a value. `get user:1.name` or `get tags[0]` reads inside a document. |
| `has [type] <key>` / `typeof <key>` | Existence and type checks (no disk access). |
| `del [type] <key>` / `del all keys` | Deletes keys; `del all keys` wipes the workspace (admin). |
| `change [type] <old> to [type] <new> [value]` | Atomic rename, or migration to a new value and type. |
| `bump [type] <key> add\|set <sec>` | Extends (`add`) or resets (`set`) a TTL. |
| `print <expr>, <expr>` | Arithmetic (`+ - * / %`, parentheses), key lookups, `"${expr}"` interpolation. |

### Queries

| Command | Description |
| :--- | :--- |
| `scan [pattern] [limit N]` | Keys with type, size, modification date, TTL and storage flags. |
| `keys [like <pattern>] [from <A> to <B>] [limit N] [reverse]` | Key names only. |
| `range <A> to <B> [reverse] [limit N] [return f1, f2]` | Keys and values in an inclusive range (`get range ...` also works). |
| `json get\|set\|delete <key> <.path> [value]` | Reads or edits one field of a document. |
| `get where <pattern>.<field> <op> <value> [limit N] [return f1, f2]` | Filters documents. Operators: `= != > >= < <=`. |
| `count where ...` / `sum where ...` | Aggregates over the matching documents. |
| `create index on <pattern> (<field>)` | Secondary index; maintained on every write. Also `drop index on ...`, `indexes`. |
| `explain <query>` | Shows the strategy: index scan, prefix scan, full scan, range scan or point lookup. |
| `convert <key> to msgpack\|json` | Changes how a document is encoded on disk. |

Patterns use `*` and `?`. In a condition the field starts at the first dot after the last wildcard: `user:*.address.city = "Kinshasa"`.

### Transactions

```text
begin
set user:1 "Alice"
savepoint sp1
set user:2 "Bob"
rollback to sp1      -- user:2 is discarded, user:1 stays
commit
```

Writes are buffered until `commit`, which applies them as one atomic batch: after a crash either all of them are present or none. A command that fails inside a transaction rolls it back. If another session modified one of the keys since `begin`, `commit` fails with `ERR_TRANSACTION_CONFLICT`. `transaction status` shows the open transaction. Inside a transaction, `get`, `has`, `typeof`, `json` and `print` see your pending writes; `range`, `keys`, `scan` and `where` show committed data only.

### Maintenance

| Command | Description |
| :--- | :--- |
| `stats` | Keys, sizes, fragmentation, memory, compression ratio, startup mode, checksum errors, replication. |
| `compact` | Rewrites the data file without dead records. Reads and writes continue meanwhile. |
| `autocompact on\|off\|status\|threshold <N>` | Background compaction when fragmentation reaches N % (default 30). |
| `check` / `check repair` | Verifies every record; `repair` saves a `.bak` copy and removes corrupted entries. |
| `hint info` | State of the index snapshot. |
| `compress lz4\|zlib\|none` | Re-encodes the whole workspace and sets the algorithm for new writes. |

### Workspaces and configuration

| Command | Description |
| :--- | :--- |
| `workspace list \| current \| switch <name> \| create <name> [path] \| remove <name>` | Isolated databases. |
| `config list \| get <key> \| set <key> <value>` | Reads or edits `kiva.conf` (`section.key` when a name is ambiguous). |

`kiva.conf` is created in the home directory on first run:

```ini
[storage]
default_workspace = <home>/data
auto_compact = true
compact_threshold = 30
compression = none            # none, lz4, zlib
sync_writes = false           # true: fsync on every write
snapshot_interval = 300       # seconds between index snapshots...
snapshot_every = 1000         # ...or after this many modifications

[workspaces]
projet_a = /home/user/dev/project_a/db

[network]
port = 2911
bind = 127.0.0.1

[security]
auth_required = false

[server]
threads = 8                   # commands executed in parallel
max_clients = 1000

[http]
enabled = true
port = 8080
rate_limit = 600              # requests per minute per IP, 0 = unlimited
cors = true

[replication]
role = master                 # master, replica
master_host =
master_port = 2911
master_user = root
master_password =
sync_mode = async             # async, sync
min_slaves_ack = 1

[scripting]
timeout_ms = 5000
memory_limit_mb = 64
```

### Users and security

```text
create user "fordi" identified by "mon_pass" [role admin|editor|viewer]
modify user "fordi" password "new_pass"
modify user "guest" role "editor"
grant read on workspace "projet_commun" to "guest"
grant write on all to "fordi"
revoke write from "guest"
drop user "guest"
users | whoami | auth <user> <password>
audit last 20 | audit since "2026-05-01" | audit failed-logins
```

* **Roles**: `admin` (everything), `editor` (read and write data, transactions, scripts), `viewer` (read only).
* **Workspaces**: an admin can use every workspace. Other users get a personal workspace (`u_<name>`) and whatever was granted to them.
* A built-in `root` administrator is created with an **empty password**. Authentication is off by default, which suits a local shell. To secure an instance:

```text
modify user "root" password "<a strong password>"
config set auth_required true
```

* `kivadb serve` refuses to listen on a non-loopback address while authentication is off or root has no password. An account with an empty password can only log in locally.
* Sensitive actions are written to `audit.log` in the home directory, with passwords masked.
* `kivadb --skip-auth` starts without authentication for maintenance.

### Scripting (Lua 5.4)

```text
eval 'return kiva.get("user:1").name'
eval 'kiva.set("visits", (kiva.get("visits") or 0) + 1)'
script load "increment.lua" as "inc"
script run "inc" "user:1" 5
script list | script show "inc" | script delete "inc"
create trigger audit on set "user:*" run "audit_log"
triggers | drop trigger audit
bench script "inc" iterations 10000
```

Scripts can call `kiva.get`, `kiva.set(key, value [, ttl])`, `kiva.del`, `kiva.has`, `kiva.typeof`, `kiva.scan([pattern])` and `kiva.incr(key [, delta])`; arguments arrive in `ARGV`. Documents become Lua tables and back.

* **Atomic**: all writes of a script are committed together, or not at all if it raises an error.
* **Sandboxed**: no `io`, `os`, `require`, `load` or `dofile`; time and memory limits from `[scripting]`.
* **Triggers** run a stored script after a key is created (`set`), overwritten (`update`) or deleted (`del`), with `ARGV = {event, key}`. They run asynchronously and do not fire each other.
* `kivadb --lua-debug` prints the lines a script executes.

## Server mode

```bash
kivadb serve
```

### TCP (port 2911)

The same port speaks two protocols:

* **Plain text**: one command per line, exactly as in the shell. Works with `telnet` or `nc`.
* **Binary**: `[length u32][code u16][payload]`, big-endian, `length` covering code and payload. A connection is binary when its first byte is `0x00`.

| Request | Code | Payload | Reply |
| :--- | :--- | :--- | :--- |
| HELLO | 1 | - | version, whether login is required |
| AUTH | 2 | `user \0 password` | welcome text or error |
| EXEC | 3 | a command line | output lines, each prefixed `+` (normal) or `-` (error) |
| PING | 4 | - | `PONG` |
| GET | 5 | key | `type u8` + value bytes (JSON for documents) |
| SET | 6 | `type u8, ttl u32, key_len u16, key, value` | `OK` |
| DEL | 7 | key | `1` or not found |

Reply status: `0` ok, `1` error, `2` not found, `3` connection closing, `9` change notification.

### REST API (port 8080)

| Route | Description |
| :--- | :--- |
| `GET /v1/key/{key}` | Reads a key. `?path=.field` reads inside a document. |
| `POST /v1/key/{key}` | Creates or replaces. Body: any JSON value, or `{"value": ..., "ttl": seconds}`. |
| `DELETE /v1/key/{key}` | Deletes a key. |
| `GET /v1/scan`, `/v1/keys`, `/v1/range` | Listings (`pattern`, `from`, `to`, `limit`, `reverse`). |
| `GET /v1/stats` | Engine statistics. |
| `POST /v1/exec` | Runs any shell command: `{"command": "..."}`. |
| `GET /metrics` | Prometheus metrics. |
| `GET /ws?pattern=user:*` | WebSocket stream of key changes, as JSON. |
| `GET /docs`, `/openapi.json`, `/health` | Interactive documentation, OpenAPI description, liveness. |

```bash
curl -X POST http://localhost:8080/v1/key/user:2 -d '{"value": {"name": "Bob"}, "ttl": 3600}'
curl -u fordi:mon_pass http://localhost:8080/v1/key/user:2
```

Responses are JSON: `{"status": "ok", "data": ..., "timestamp": ...}` or `{"status": "error", "error": "..."}`. With `auth_required`, send HTTP Basic credentials; add `?workspace=<name>` to work outside your home workspace. HTTPS is not built in: put a TLS-terminating reverse proxy in front when exposing the API.

### Change notifications

`subscribe user:*` in a shell (or on a TCP connection, or through `/ws`) announces every `set`, `update` and `del` on matching keys of the current workspace. `unsubscribe` stops it.

### Replication

On the replica:

```text
replica of 192.168.1.10 2911
```

The replica drops its local default workspace, receives a full snapshot, then follows the live stream and reconnects by itself if the link breaks. It is read-only.

| Command | Description |
| :--- | :--- |
| `replica status` | Role, connected replicas and their lag, or state of the link to the master. |
| `replica start \| stop \| sync` | Start following the configured master, stop, or force a full resynchronisation. |
| `replica promote` | Turn a replica into a master. |
| `wait <N> [timeout_ms]` | Block until N replicas have everything written so far. |
| `failover to <replica>` | Hand the master role to a replica; the old master becomes its replica. |

With `sync_mode = sync`, every write waits for `min_slaves_ack` acknowledgements. Replication covers the **default workspace**; when the master requires authentication, set `master_user` / `master_password` on the replica (an admin account).

## Storage format

Each workspace is a directory:

| File | Role |
| :--- | :--- |
| `store.kiva` | The data: an append-only log of records. |
| `store.kiva.hint` | Index snapshot for fast startup. Safe to delete. |
| `store.kiva.meta` | Secondary index definitions. |
| `store.kiva.lock` | Guards against two processes opening the same files. |
| `store.kiva.bak` | Backup taken before a repair. |

All integers are little-endian.

**File header (16 bytes)**

| Field | Size | Description |
| :--- | :--- | :--- |
| magic | 4 | `KIVA` |
| format | 4 | Storage format number, `1` |
| file_id | 8 | Random identifier; changes at every compaction so a stale hint is never trusted |

**Record (28-byte header + key + value)**

| Field | Type | Size | Description |
| :--- | :--- | :--- | :--- |
| crc32 | `u32` | 4 | Checksum of everything that follows in the record |
| k_size | `u16` | 2 | Key length |
| v_size | `u32` | 4 | Stored value length |
| type | `u8` | 1 | 1 string, 2 number, 3 boolean, 4 null, 5 array, 6 object, 7 bytes |
| flags | `u8` | 1 | bits 0-1 compression (0 none, 1 LZ4, 2 Zlib); bit 2 MessagePack; bit 5 part of a batch; bit 6 commit of the batch; bit 7 tombstone |
| timestamp | `i64` | 8 | Write time (Unix seconds) |
| expires_at | `i64` | 8 | Expiry time, `0` = never |
| key | bytes | k_size | UTF-8 |
| value | bytes | v_size | Compressed values start with their original length (`u32`) |

Records of a transaction carry the batch flag; they take effect only once the record with the commit flag is on disk, which is what makes a commit atomic.

**Durability.** Writes go to the operating system immediately and are flushed to the disk once per second, so a crashed or killed process loses nothing. A power failure can lose the last second of writes; set `sync_writes = true` to flush on every write instead.

## Using KivaDB as a library

```rust
use kivadb::{Engine, EngineOptions, Value};

let db = Engine::open("data/store.kiva", EngineOptions::default())?;
db.put("greeting", &Value::Str("hello".into()), 0, false)?;
if let Some((value, _)) = db.get("greeting")? {
    println!("{}", value.display());
}
```

`Instance` and `Session` give access to the full command language (`session.execute("get greeting")`), users and workspaces.

## Source layout

| Path | Contents |
| :--- | :--- |
| `src/engine.rs`, `format.rs`, `hint.rs`, `compress.rs` | Storage engine, record format, index snapshot, compression |
| `src/value.rs`, `jsonpath.rs` | Types and document paths |
| `src/lexer.rs`, `expr.rs`, `exec.rs`, `exec_admin.rs` | Command language |
| `src/session.rs`, `instance.rs`, `auth.rs`, `config.rs` | Sessions, transactions, users, configuration, workspaces |
| `src/server.rs`, `proto.rs`, `http.rs`, `repl.rs` | TCP server, wire protocol, REST API, replication |
| `src/script.rs` | Lua sandbox |
| `src/shell.rs`, `main.rs` | Interactive shell and command line |
| `tests/` | Engine, command and network test suites |

## Scope of this release

KivaDB 1.0.0 is a single-node database with replicas. The following are **not** part of it: built-in TLS, automatic leader election, and clustering (sharding, quorum reads and writes, Raft consensus, distributed queries).

## License

This repository is published under and protected by the **[FomaDev Public License (FPL)](LICENSE)**.

* **Binary Utilization**: Permissions are granted to execute compiled KivaDB CLI binaries within personal, academic, and industrial production environments free of charge.
* **Source Modification**: Redistribution, compilation, or integration of source fragments into third-party commercial applications or competitive data frameworks requires an official paid licensing agreement.
* **Contributions**: Forks are exclusively permitted for creating pull requests targeting the primary branch of the official repository.
