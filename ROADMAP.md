# KivaDB roadmap

What KivaDB contains is described in [DOCUMENTATION.md](DOCUMENTATION.md), and each version's changes in [CHANGELOG.md](CHANGELOG.md). This page lists what was considered and left for later, and why. Each item says what it would really take, so that decisions are made on facts.

## Delivered in 1.0.0 (for reference)

* Key-value, JSON documents, secondary indexes, queries and aggregates
* **Vector search** with in-memory vector indexes (exact search)
* **Redis protocol** on port 6379 (strings, counters, expiry, hashes, lists, transactions)
* **Optional value cache** in memory
* **Server-Sent Events** on `/v1/stream`, besides WebSocket
* **Official Node.js driver** (`drivers/node`), Rust client in the crate
* Shell with persistent history and Tab completion
* Kiva Studio: data browser, query builder with execution plans, similarity search, activity feed with alerts, users, server health
* Ready-made executables for Windows x64, Linux x64 and macOS Apple Silicon, and the Docker image `fomadev0/kiva-db`, published automatically for every version

## Delivered since (for reference)

* **1.0.1** `kivadb start` shows every port it opened
* **1.1.0** `kivadb update`, a daily check for new releases, `install.sh`
* **1.2.0** Editors may compact their own personal workspace
* **1.3.0** Redis compatibility complete enough to replace Redis as short-lived memory: Lua scripts (`EVAL`), RESP3, numbered databases, `EXPIRE` options, atomic counters across connections
* **1.4.0** **TLS** on every port with certificates checked by fingerprint, commands over several lines, `.kva` command files, `install.ps1` for Windows

## Next, in order of value

| Item | What it brings | What it takes |
| :--- | :--- | :--- |
| **Python driver** (PyPI) | Python is the language of AI work, where vector search matters most | A port of the Node.js driver (~300 lines) and a test environment with Python, which was not available when 1.0.0 was built. Meanwhile, Python's `redis` package works through the Redis protocol, and the REST API works with `requests` |
| **Approximate vector index (HNSW)** | Similarity search over millions of vectors in milliseconds | A graph index kept in memory and updated on every write, plus its persistence. A Rust implementation (or the `hnsw_rs` crate) fits the current design |
| **Go and Java drivers** | Reach backend teams in those languages | Same protocol work as the Node.js driver, per language |
| **More Redis commands** | Sets, sorted sets (leaderboards), publish/subscribe | New value types or mappings, and their tests |
| **Kiva Studio on mobile** | Check alerts and act from a phone | Flutter already targets Android and iOS; the work is a phone-sized layout and store publishing. Push notifications when the app is closed need a server and Apple/Google accounts |
| **Kiva Studio on the web** | Use Studio from a browser, without installing anything | Browsers cannot open raw TCP sockets: the web version must go through the REST API, which then needs a session endpoint for transactions and subscriptions. Flutter's web target or a separate web app are both possible |
| **Saved passwords in Kiva Studio** | One-click connection to remote servers | Storing them in the operating system's keychain (Windows Credential Manager, macOS Keychain, Secret Service on Linux), never in a file |
| **More ready-made builds** | Executables for Linux on ARM (Raspberry Pi, ARM servers) and Intel Macs, and a Docker image for ARM | More entries in the release workflow (cross-compilation or ARM runners) and a multi-architecture Docker build (`docker buildx`) |
| **Kiva Studio installers** | Install Studio without Flutter: a Windows installer, a macOS app, a Linux package, with its own icon | A release workflow for the `kiva-studio` repository; signing the macOS and Windows builds needs Apple and Microsoft certificates |

## Considered and not planned, with reasons

| Idea | Why not now |
| :--- | :--- |
| **PostgreSQL protocol (PgWire) so that Prisma or DBeaver connect "without changing code"** | These tools do not only send queries: they read PostgreSQL's catalog (`pg_catalog`), expect tables, columns and SQL types, and generate SQL. Answering them means writing a SQL engine with a relational model, not a protocol adapter. That is a different product |
| **"Replace MySQL"** | KivaDB is not relational: no joins, no SQL, no foreign keys. It is a strong fit for caches, sessions, documents, AI memory, configuration and event data; applications built around relations and reports are better served by a relational database, possibly next to KivaDB |
| **RocksDB or Faiss (C++) inside KivaDB** | RocksDB would replace KivaDB's own storage engine rather than add to it, and both bring large C++ builds that make KivaDB harder to compile on every system. The Rust engine is the project's core and its point |
| **Assembly for the CRC32 checksums** | The `crc32fast` crate already uses the processor's dedicated instructions (SSE4.2 / PCLMUL on Intel and AMD, CRC on ARM). Hand-written assembly would add risk and no speed |
| **Plugins in WebAssembly, scripts in JavaScript or Python** | Lua already provides sandboxed, atomic scripts and triggers. A WebAssembly runtime (Wasmtime) is a large dependency; it becomes worth it when there is demand for plugins written in other languages |
| **"Time travel" (see a key's past values)** | The log does keep old versions, but compaction deletes them on purpose to reclaim space. Time travel needs a retention policy (keep N versions or N days), an index of versions, and disk budgeting |
| **MQTT, gRPC, GraphQL listeners** | Each is a protocol to maintain. The REST API, Server-Sent Events, WebSocket, the binary protocol and the Redis protocol already cover web, mobile, scripts and IoT gateways |
| **Clustering (sharding, Raft, quorum, distributed queries)** | A distributed database is a multi-year effort to get right. Replication with failover covers safety and read scaling for a single node's worth of data |
| **Kiva Studio cloud accounts** | Syncing settings and sending notifications through a FomaDev service means running that service: servers, security, privacy obligations and costs. Local remembered connections and the in-app activity feed cover the need without it |

## About a few claims

Some descriptions of KivaDB found online overstate things. For the record:

* **"Rust eliminates 100 % of memory bugs."** Safe Rust prevents whole classes of memory errors (buffer overflows, use-after-free, data races). It does not prevent logic errors, and code that binds to C (such as the embedded Lua) is outside that guarantee. KivaDB's reliability comes from the language *and* from its tests (crash recovery, corruption detection, concurrency).
* **"Faster than Redis."** Redis keeps everything in memory and is extremely optimised. KivaDB writes every change to disk before answering and checks every record's checksum on read. It is fast, and durable by default; it is not designed to beat Redis on raw in-memory speed.
* **"Multi-model."** True for key-value, documents and vectors, which share the same keys, files, transactions and permissions. Time series are supported in the sense that sorted keys make time ranges cheap (`range "log:2026-10-15T09" to "log:2026-10-15T10"`), not through a dedicated time-series engine.
 