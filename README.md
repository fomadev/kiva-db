<p align="center">
  <img src="ban.png" alt="KivaDB Logo">
</p>

# KivaDB

KivaDB is a multi-model database you can start in one command: typed key-value data, JSON documents with indexes and queries, and vector search for AI, in a single executable written in Rust. You talk to it from its shell, from any language (its own protocol, the Redis protocol, or HTTP), or from Kiva Studio, its desktop application.

**Current release: 1.0.0** &nbsp;|&nbsp; **[Read the full documentation → DOCUMENTATION.md](DOCUMENTATION.md)**

## What it does

* **Stores typed data**: strings, numbers, booleans, null, arrays, objects (JSON documents) and raw bytes.
* **Keeps it safe**: every record is checksummed, a crash or a killed process loses nothing, and transactions are atomic.
* **Finds it fast**: keys are kept sorted (range queries, pagination), documents can be filtered, counted, summed and indexed.
* **Understands AI embeddings**: `get similar doc:*.embedding to [...]` finds the closest documents (cosine, dot or Euclidean), with in-memory vector indexes.
* **Speaks Redis**: applications and libraries written for Redis connect to port 6379 unchanged, for strings, counters, expiry, hashes, lists and transactions.
* **Runs as a service**: `kivadb start` puts the database in the background on port **2911**; shells, programs, the REST API (port 8080, with WebSocket and Server-Sent Events) and Redis clients (port 6379) all work on the same live data.
* **Grows with you**: users and roles, isolated workspaces, an optional in-memory value cache, compression, replication to other machines, Lua scripts and triggers, change notifications, Prometheus metrics, Docker image, official Node.js driver.

## Install

You need [Rust](https://rustup.rs) and a C compiler (used once, for the embedded Lua).

```bash
cargo build --release
```

The executable is `target/release/kivadb` (`kivadb.exe` on Windows). Put it somewhere in your `PATH`.

## Five-minute tour

Start the database, then open a shell:

```bash
kivadb start
```

```bash
kivadb
```

```text
kiva> set name "Fordi" and age 30
OK: name saved.
OK: age saved.
kiva> set user:1 {"name": "Alice", "role": "admin", "address": {"city": "Kinshasa"}}
OK: user:1 saved.
kiva> get user:1.address.city
user:1.address.city: Kinshasa [string]
kiva> count where user:*.role = "admin"
Count: 1
kiva> print "Next year ${name} will be ${age + 1}"
Next year Fordi will be 31
```

The same data, from a program or the command line:

```bash
curl http://localhost:8080/v1/key/user:1
```

```text
kivadb status     is the database running?
kivadb stop       stop it
kivadb --help     every option
```

Type `help` inside the shell for the list of commands.

## Documentation

Everything is explained step by step, with examples, in **[DOCUMENTATION.md](DOCUMENTATION.md)**: installation, starting and stopping, every command, data types, queries and indexes, transactions, users and security, the server protocols, connecting from Rust / Node.js / Python / curl, replication, Lua scripting, the file format, and troubleshooting.

| If you want to... | Read |
| :--- | :--- |
| Get going from zero | [Chapters 0 to 3](DOCUMENTATION.md#chapter-0--introduction) |
| Learn the commands | [Chapters 4 to 12](DOCUMENTATION.md#chapter-4--syntax-rules) |
| Connect an application | [Chapters 18 to 21](DOCUMENTATION.md#chapter-18--the-tcp-server) |
| Run it in production | [Chapters 14 to 17, 22 and 24](DOCUMENTATION.md#chapter-14--configuration) |
| Host it: Linux, Windows, macOS, Docker | [Chapter 29](DOCUMENTATION.md#chapter-29--deploying-kivadb-on-a-server) |
| Use vectors, Redis clients, the value cache | [Chapters 30 to 32](DOCUMENTATION.md#chapter-30--vector-search-for-ai) |
| Look a command up quickly | [Chapter 27, the cheat sheet](DOCUMENTATION.md#chapter-27--command-cheat-sheet) |

## Project layout

| Path | Contents |
| :--- | :--- |
| `src/` | The database: storage engine, command language, server, replication, scripting |
| `tests/` | Automated tests (storage, commands, network) - run them with `cargo test` |
| `drivers/node/` | The official Node.js driver (npm package `kivadb`) |
| `examples/nodejs/` | A dependency-free Node.js client |
| `DOCUMENTATION.md` | The complete manual |
| `CONTRIBUTING.md` | How to contribute |

## Scope of this release

KivaDB 1.0.0 is a single-node database with replicas. Built-in TLS, automatic leader election, clustering, SQL / the PostgreSQL protocol and approximate vector indexes are not part of it; [ROADMAP.md](ROADMAP.md) explains what comes next and why.

## License

This repository is published under and protected by the **[FomaDev Public License (FPL)](LICENSE)**.

* **Binary Utilization**: Permissions are granted to execute compiled KivaDB CLI binaries within personal, academic, and industrial production environments free of charge.
* **Source Modification**: Redistribution, compilation, or integration of source fragments into third-party commercial applications or competitive data frameworks requires an official paid licensing agreement.
* **Contributions**: Forks are exclusively permitted for creating pull requests targeting the primary branch of the official repository.
