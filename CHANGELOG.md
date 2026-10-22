# Changelog

Every version of KivaDB, newest first. Version numbers follow [Semantic Versioning](https://semver.org): MAJOR for incompatible changes, MINOR for new features, PATCH for compatible improvements and fixes.

## 1.5.0 - 2026-10-22

Security, availability and AI: data encrypted on disk, a new master elected by itself when the old one dies, backups in one command, an approximate vector index and a semantic cache for large AI workloads, and Redis sets, sorted sets and publish/subscribe.

### Added

* `set <key> <value> if not exists`: creates the key only when it is absent; otherwise the command succeeds with `OK: <key> skipped (already exists)` instead of an error. Works with a type, `ttl` and `and` chains.
* `del <key> if exists`: deletes the key when present; otherwise prints `OK: <key> skipped (does not exist)`. Both clauses are case-insensitive and make `.kva` files safe to run again. DOCUMENTATION 6.1, 6.5 and 3.10.
* **Redis publish/subscribe**: `PUBLISH`, `SUBSCRIBE`, `UNSUBSCRIBE`, `PSUBSCRIBE`, `PUNSUBSCRIBE` (Redis glob patterns), `PUBSUB CHANNELS | NUMSUB | NUMPAT`, in RESP2 and RESP3 (push frames). An application with several instances can tell all of them that something changed. Channels belong to the user's database 0 workspace; messages published inside `MULTI` or a Lua script leave when it commits; a listener that stops reading is disconnected after 65,536 waiting messages. Checked with ioredis and node-redis. DOCUMENTATION 23.7.
* **HNSW vector index** for large collections: `create vector index on doc: (embedding) using hnsw [metric cosine|dot|l2] [m N] [ef N]`. `get similar` uses it automatically when the pattern and metric match, and compares only a small part of the vectors (on 100 000 vectors of 128 dimensions grouped by topic, 0.5 ms per search instead of 105 ms, with the same 10 results). New search options `ef N` (accuracy against speed) and `exact`; the JSON request takes `"ef"` and `"exact"` and answers `"approximate"`; the Node.js driver passes them on. The graph follows every write; deleted vectors are cleaned up by a rebuild in the maintenance task; it is saved in `<file>.graphs` and read back at start, replaying only the writes made since. DOCUMENTATION 12.5 and 12.6.
* **Encryption at rest** (AES-256-GCM, with `ring`): every record (key and value together, header authenticated), the index snapshot (`.hint`) and the HNSW graphs (`.graphs`), in every workspace and the system database. The key comes from `KIVADB_ENCRYPTION_KEY` or a file named by `encryption.key_file`, never from the data folder; `kivadb keygen` creates one. `compact` encrypts data written before the key; `KIVADB_ENCRYPTION_OLD_KEY` / `encryption.old_key_file` allow a key rotation; `encryption.required` refuses to start without a key and refuses records in clear. The `encryption` command shows the key id and what is still in clear. With encryption on, the local shell keeps no history file. DOCUMENTATION 16.12.
* **Redis sets and sorted sets**: `SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SMISMEMBER`, `SCARD`, `SPOP`, `SRANDMEMBER`, `SMOVE`, `SINTER` / `SUNION` / `SDIFF` (and `STORE`), `SINTERCARD`, `SSCAN`; `ZADD` (`NX`, `XX`, `GT`, `LT`, `CH`, `INCR`), `ZINCRBY`, `ZREM`, `ZSCORE`, `ZMSCORE`, `ZCARD`, `ZCOUNT`, `ZLEXCOUNT`, `ZRANK` / `ZREVRANK` (`WITHSCORE`), `ZRANGE` (`BYSCORE`, `BYLEX`, `REV`, `LIMIT`, `WITHSCORES`), `ZRANGESTORE`, `ZREVRANGE`, `ZRANGEBYSCORE`, `ZRANGEBYLEX` and their reverse forms, `ZPOPMIN` / `ZPOPMAX`, `ZREMRANGEBYRANK` / `SCORE` / `LEX`, `ZRANDMEMBER`, `ZSCAN`, `ZUNION` / `ZINTER` / `ZDIFF` (and `STORE`, `WEIGHTS`, `AGGREGATE`), `ZINTERCARD`. Stored as marked KivaDB objects (saved, replicated, encrypted), `TYPE` answers `set` / `zset`, RESP3 doubles and sets. Checked with ioredis and node-redis. DOCUMENTATION 23.8.
* **`kivadb backup` and `kivadb restore`**: `kivadb backup --output <file.tar.gz> [-w <workspace>]` saves the whole home (configuration, users, every workspace, audit log, TLS certificate) or one workspace, with a SHA-256 per file in a manifest and `<file>.sha256` for the archive. It works while KivaDB runs (the server makes the backup; each data file is copied as it was at one instant, writes go on, compactions wait) or stopped. `kivadb restore <file.tar.gz> [--target <dir>] [--as <name>] [--check]` checks the archive, every file and every record before writing anything, then restores a home into an empty folder (with `kiva.conf` pointing at its new place) or a workspace under its name or another; it refuses while KivaDB runs on that home. Shell: `backup to "<file>" [workspace <name>]` (admin). DOCUMENTATION 17.7.
* **Automatic failover** (`failover.auto`): the master sends heartbeats listing the group's nodes; a replica that hears nothing for `failover.down_after_ms` checks the master itself, asks the other replicas, and a new master is elected only when a majority of the group (the master counted; `failover.quorum` to change it) sees the master down. The most up-to-date replica wins (then `failover.priority`, 0 = never); votes are given once per epoch to a candidate at least as up to date. The new master tells the other replicas, which resynchronise from it; an old master that comes back finds the newer epoch and becomes a replica. `failover status`, `READONLY` errors that name the master, `master_host` / `master_port` in Redis `INFO`. `replica promote` and `failover to` open a new epoch too. DOCUMENTATION 25.8.
* **Semantic cache**: `semantic lookup | store | stats | clear` keep answers (of a language model, say) next to the embedding of their question, and give them back for a question at cosine similarity >= `semantic.cache.threshold` (0.98), or for the same text word for word. Named caches per workspace, HNSW search, TTL (`semantic.cache.ttl`, per entry), invalidation as soon as a key matching a `depends` pattern is written or deleted, `semantic.cache.max_entries`. With `semantic.cache.enabled`, `get similar` reuses the results of a near-identical recent query. Structured request `semantic`, `cached` in `similar` answers, `semanticLookup` / `semanticStore` in the Node.js driver. In memory, not saved. DOCUMENTATION 12.8.

### Changed

* A write refused on a replica names the master: `READONLY: this node is a replica of <host:port>; ...`.
* `config set required ...` (without its section) is ambiguous since `encryption.required` exists next to `tls.required`: write `config set tls.required true` or `config set encryption.required true`, as the documentation does.
* The distance functions of vector search are vectorised: exact searches are faster too.

### Fixed

* Reopening a database right after it was closed could fail with "database is locked" on Linux, while another process was being started (it inherits open files for an instant). KivaDB now waits up to half a second for such a lock.

## 1.4.0 - 2026-10-20

Encrypted connections, commands over several lines with `.kva` command files, and a one-command installer for Windows.

### Added

* **Encrypted connections (TLS)** on every port, next to plain ones: 2911 (shells, Kiva Studio, drivers), 8080 (HTTPS) and 6379 (`rediss://`). KivaDB recognises a TLS connection by its first byte.
* A certificate created on first start in `<home>/tls/`, or your own (`tls.cert`, `tls.key`). `kivadb tls` shows it and its SHA-256 fingerprint; `kivadb tls renew` creates a new one; `tls.hosts` adds names or addresses to it.
* `tls.required`: refuse plain connections from other machines (the machine itself may still connect in clear). Refused clients get a message in their own protocol.
* Shell: `kivadb --connect host:port --tls`. A new server's fingerprint is shown and, at a terminal, the user is asked to trust it; trusted servers are kept in `known_servers`. `--tls-fingerprint` for scripts. A remote connection without `--tls` prints a warning.
* Rust client: `Client::connect_tls`. Node.js driver: the `tls` option, with `fingerprint`. Replicas: `replication.master_tls` and `replication.master_fingerprint`.
* DOCUMENTATION 16.11.
* **Commands over several lines** in the shell: while a `{` or `[` is open, Enter goes on to a ` ...>` line instead of failing with "Unclosed bracket"; the command runs when the document closes. The history keeps it whole.
* **`.kva` command files**: commands as in the shell, documents over several lines, `#` and `--` comments. `kivadb run file.kva` runs them (also on a remote server with `--connect`), reports a failing command with its file and line and stops there (`--keep-going` to go on); `source "file.kva"` does the same in the shell. DOCUMENTATION 3.9 and 3.10.
* **`install.ps1`**: installs the latest release on Windows in one PowerShell command, `irm https://raw.githubusercontent.com/fomadev/kiva-db/main/install.ps1 | iex`: SHA-256 check, `%LOCALAPPDATA%\Programs\KivaDB` without administrator rights, the folder added to the user's `PATH`. `KIVADB_INSTALL_DIR`, `KIVADB_VERSION`, `KIVADB_NO_MODIFY_PATH`.

### Changed

* Commands piped into the shell (`kivadb < file`) end with exit code 1 when one of them failed (it was always 0).

## 1.3.0 - 2026-10-18

Redis compatibility, made complete enough to replace Redis where it serves as short-lived memory (sign-up codes, locks, rate limits, sessions). Checked with the current ioredis, node-redis, rate-limiter-flexible and rate-limit-redis, now part of the CI (`tests/redis-clients`).

### Added

* Lua scripts through Redis: `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`, `SCRIPT LOAD | EXISTS | FLUSH`, with `redis.call`, `redis.pcall`, `redis.error_reply`, `redis.status_reply`, `redis.sha1hex`. Scripts run in the Lua sandbox, all or nothing.
* RESP3: `HELLO 3` is accepted (node-redis 5 and later use it), with maps and nulls.
* Numbered databases: `SELECT 1` to `15` (or `/n` in a URL) use the workspaces `db1` to `db15`; database 0 is the user's own workspace.
* `EXPIRE`, `PEXPIRE`, `EXPIREAT`, `PEXPIREAT` accept `NX`, `XX`, `GT` and `LT`.
* `GETEX`, `EXPIRETIME`, `PEXPIRETIME`, `TIME`.
* DOCUMENTATION 23.5 (Lua scripts) and 23.6 (replacing Redis in an application).

### Fixed

* Commands from several Redis connections could interleave: two clients running `INCR` on the same key at the same moment could lose a step, which weakens a rate limiter. Redis commands now run one at a time, as in Redis.
* `EXPIRE ... NX` (and the other options) were ignored, so a rate-limit window restarted at every attempt.

## 1.2.0 - 2026-10-18

### Changed

* `compact` is no longer reserved to administrators in every case: an `editor` may now compact their own personal workspace (`u_<name>`). Everywhere else, including workspaces a user can write to through a grant, it still needs the admin role, because it rewrites a whole shared file. Viewers still cannot run it. The refusal message says where it is allowed.

## 1.1.0 - 2026-10-18

### Added

* `kivadb update` installs the latest release in place of the running executable, on Windows, Linux and macOS. The download is checked against the release's `SHA256SUMS.txt` and the new executable is run once before it replaces the old one; on any problem nothing is changed. `kivadb update --check` only tells whether a new version exists.
* A discreet check for new releases: in a terminal, KivaDB looks at GitHub at most once a day, in the background, and mentions a new version when the command ends. Scripts, services, `kivadb serve` and containers never trigger it. `KIVADB_NO_UPDATE_CHECK=1` turns it off.
* `install.sh`: installs the latest release on Linux or macOS in one command, `curl -sSL https://raw.githubusercontent.com/fomadev/kiva-db/main/install.sh | bash`.
* `KIVADB_RELEASES_URL` points `kivadb update` to a mirror of the releases.

## 1.0.1 - 2026-10-16

### Improved

* `kivadb start` now shows every listener it started, not only the TCP port:

  ```text
  KivaDB started (pid 13052).
    TCP port : 2911
    REST API : http://127.0.0.1:8080
    Redis    : port 6379
  ```

  When the REST API or the Redis port cannot start (another program already uses the port), the reason is shown on those lines instead, and KivaDB runs without it.

### Internal

* The test suites read the version from `Cargo.toml` instead of repeating it.

## 1.0.0 - 2026-10-15

First release: typed key-value and document database in one executable, with secondary indexes and queries, vector search, transactions, workspaces, users and roles, compaction, compression, replication with failover, Lua scripts and triggers, change notifications, REST API, Redis protocol, Prometheus metrics, Docker image and the Node.js driver. See [DOCUMENTATION.md](DOCUMENTATION.md).
