# KivaDB 2.0 — implementation matrix

Where each item of the 2.0 plan stands, checked against the code rather than assumed. Every row gives its proof (a file, a test, a section of [DOCUMENTATION.md](../../DOCUMENTATION.md)) and what is left. This page is kept up to date by every pull request of the 2.0 work, and removed or archived once 2.0.0 is released.

Status: **done** (in `main`, tested and documented), **existing** (already in 1.7.1, verified), **partial**, **planned** (in a coming pull request of 2.0), **deferred** (after 2.0, with its reason), **not planned** (rejected in [ROADMAP.md](../../ROADMAP.md), with its reason).

## Baseline

* Version in `Cargo.toml`: `2.0.0`, the release, not tagged yet; `v2.0.0-rc.1` was published without its Docker image, `v2.0.0-rc.2` on 2026-10-29, `v2.0.0-rc.3` and `v2.0.0-rc.4` on 2026-11-20, `v2.0.0-rc.5` on 2026-11-21, `v2.0.0-rc.6` to `v2.0.0-rc.10` on 2026-11-22 ([RELEASE.md](RELEASE.md)); what 2.0.0 brings is the section "2.0.0" of [CHANGELOG.md](../../CHANGELOG.md), and the audit before it [docs/testing/production-readiness-2.0.0.md](../testing/production-readiness-2.0.0.md).
* `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings` and `cargo test` pass on Windows, Linux and macOS (CI), with no known failure.
* Storage in 1.7.1: an append-only log per workspace (`store.kiva`), file header `KIVA` + format version 1 (`src/format.rs`), CRC32 on every record, a hint file with its own version (`src/hint.rs`), index definitions in `store.kiva.meta`, HNSW graphs in `store.kiva.graphs`. Type codes 1 to 7 are those of 1.x and are never reused.

## P0

| # | Action | Item | Status | Proof | Left |
| :- | :- | :- | :- | :- | :- |
| 01 | add | `date` | done | `src/time.rs`, `src/value.rs` (type code 8), test `dates_times_and_datetimes`, [5.8](../../DOCUMENTATION.md#58-date-datetime-time) | |
| 02 | add | `time` | done | type code 10; a time of day, never a duration; leap seconds refused | |
| 03 | add | `datetime` | done | type code 9; an instant in UTC to the millisecond; the offset given is applied, not kept ([decision 2](DECISIONS.md#2-a-datetime-without-an-offset-is-read-in-the-sessions-zone)) | |
| 04 | add | time functions | done | `now()`, `today()`, `current_time`, `+ 30 days`, `year()` to `weekday()` in `print`, conditions and `group by`, [11.1](../../DOCUMENTATION.md#111-the-condition) | `date_diff` (see 40) |
| 05 | modify | `src/value.rs` | done | `Value::Date`, `DateTime`, `Time`; every `match` extended | |
| 06 | modify | serialization | done | canonical text under the new type codes ([decision 1](DECISIONS.md#1-moments-are-stored-as-canonical-text-under-new-type-codes)); JSON and Redis give ISO 8601 in UTC | |
| 07 | add | storage versioning | done | format 2 for files holding the types of 2.0, raised at the first such record, so that 1.x refuses them instead of cutting them ([decision 1](DECISIONS.md#1-moments-are-stored-as-canonical-text-under-new-type-codes)); a newer format is refused and left untouched | |
| 08 | add | migration from 1.x | done | nothing to convert in the data files ([decision 3](DECISIONS.md#3-no-kivadb-upgrade-command)); accounts migrated at the first start (`migrate_personal_workspaces`); `tests/compat.rs` | |
| 09 | strengthen | transactions | existing | [13.x](../../DOCUMENTATION.md#chapter-13--transactions), `a_batch_without_its_commit_record_never_happened`, `optimistic_conflicts_are_detected` | |
| 10 | strengthen | concurrent writes | existing | `compaction_does_not_lose_concurrent_writes`, unique indexes checked under the write lock ([11.7](../../DOCUMENTATION.md#117-unique-indexes)) | |
| 11 | strengthen | journal and recovery | existing | `a_torn_final_write_is_discarded`, `a_damaged_record_in_the_middle_is_skipped_at_startup`, `a_killed_process_loses_nothing`, `bit_rot_is_detected_and_repaired` | |
| 12 | add | `explain` | existing + done | `explain analyze` runs the query and counts index entries, keys, documents read (cache or disk), filtered out, sorted, returned, and the time of each stage; `"analyze": true` over REST; `explain` alone reads no document (`explain_reads_no_document_and_analyze_reads_them`); writes are refused. `tests/queries.rs`, [11.5](../../DOCUMENTATION.md#115-explain--see-how-a-query-will-run) | |
| 13 | modify | query optimisation | done | `order by <field> [asc\|desc]`, pages with `after <key>`, `reverse` honoured; `and` lists its smallest part and checks the others against their indexes; a unique index on several fields answers an `and` of equalities; a B-tree on the order field read in order up to the limit; limited scans read keys 256 at a time. Every plan compared with a reading of every document (`indexed_sorted_and_paged_queries_match_a_reading_of_every_document`), [measured](benchmarks/queries.md), [decision 7](DECISIONS.md#7-the-order-of-values-and-the-choice-of-a-plan) | statistics are those the indexes hold (no histogram); `$time` comparisons and the date functions other than `year()` read every document |
| 14 | strengthen | secondary and unique indexes | existing | `secondary_indexes_follow_every_change`, unique index tests; indexes of 1.7.1 used after upgrade (`tests/compat.rs`) | |
| 15 | add | integrity checker | existing + done | `check` / `check repair` ([17.4](../../DOCUMENTATION.md#174-integrity-check)); `kivadb doctor` checks every record offline | |
| 16 | add | `kivadb doctor` | done | `src/doctor.rs`, `--json`, exit code, [17.9](../../DOCUMENTATION.md#179-kivadb-doctor--a-health-check-of-the-home), `the_doctor_reads_a_home_of_1_7_1_without_changing_it` | |
| 17 | strengthen | backup and restore | existing | `kivadb backup`, `restore --check` (SHA-256 of the archive and of every file, every record), `tests/backup.rs` | |
| 18 | strengthen | authentication and permissions | done | 2.0 grants on scopes and projects, no implicit workspace ([16.4](../../DOCUMENTATION.md#164-workspaces-and-grants)), `tests/commands.rs` | |
| 19 | strengthen | TLS and encryption at rest | existing | `tests/tls.rs`, `tests/encryption.rs`; an encrypted home of 1.7.1 opens with its key and not without (`tests/compat.rs`) | |
| 20 | test | compatibility with 1.x | done | `tests/fixtures/v1.7.1*`, made by the 1.7.1 executable from `v1.7.1.kva`; `tests/compat.rs`, which also runs the real 1.7.1 in CI (Linux) to go back; a home written by the published `2.0.0-rc.2` (`tests/fixtures/v2.0.0-rc.2`, `tests/compat_rc2.rs`) one by the published `2.0.0-rc.3` (`tests/fixtures/v2.0.0-rc.3`, `tests/compat_rc3.rs`) and one by the published `2.0.0-rc.4` (`tests/fixtures/v2.0.0-rc.4`, `tests/compat_rc4.rs`), one home taken from the real 1.7.1 through the real `rc.2` to this version, and the real `rc.3` and `rc.4` reading a home this version wrote, in CI ([28.1](../architecture/NATIVE_DOCUMENTS.md#281-compatibility-with-rc2-and-the-crash-of-a-conversion-g1-g2)) | a fixture per later release |
| 39 | document | migration guide | done | [Chapter 33](../../DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20): one server, Docker, a replication group, applications, going back; the replication between 1.x and 2.x refused both ways (`replication_never_crosses_major_versions`, against the real 1.7.1 in CI) | |

## P1

| # | Action | Item | Status | Proof | Left |
| :- | :- | :- | :- | :- | :- |
| 21 | add | optional schemas | done | `src/schema.rs`, checked in the engine on every write path, `tests/schemas.rs`, [11.8](../../DOCUMENTATION.md#118-schemas-rules-for-the-documents-of-a-pattern) | |
| 22 | add | constraints and default values | done | `required`, `nullable`, `unique` (unique index), `min`/`max`, `minLength`/`maxLength`, `pattern`, `enum`, arrays and nested objects; defaults, `now()`, `today()`, `current_time` | |
| 23 | add | composite indexes | partial | unique indexes on several fields ([11.7](../../DOCUMENTATION.md#117-unique-indexes)) | non-unique composite indexes |
| 24 | add | durable change feed (CDC) | done | `src/changes.rs`, hooks in `src/engine.rs`, `tests/changes.rs`, change journal tests in `tests/network.rs`, [22.5](../../DOCUMENTATION.md#225-the-change-journal-changes-kept-read-again-from-a-cursor), [decision 5](DECISIONS.md#5-the-change-journal-is-derived-from-the-data-file) | journals are per node (no replication of cursors); schemas are per node too (see 21) |
| 25 | add | value history | not planned | [ROADMAP.md](../../ROADMAP.md): needs a retention policy and disk budgeting | |
| 26 | modify | cache invalidation | existing | `the_value_cache_serves_hot_keys_and_never_returns_stale_data`, `the_value_cache_stays_within_its_budget` | |
| 27 | strengthen | HNSW | existing | recall and `tune` tests in `tests/power.rs`, graph of 1.7.1 read back (`tests/compat.rs`) | |
| 28 | strengthen | semantic cache | existing | `tests/semantic_cache.rs`, `depends` invalidation | |
| 29 | add | Python driver | done | `drivers/python` (1.1.0, standard library only, typed; `types="extended"` gives `Decimal`, `date`, `time` and `datetime`), `tests/test_driver.py` against a real server, `.github/workflows/python-driver.yml` (Windows, Linux, macOS; Python 3.10, 3.12, 3.14; `mypy --strict`; wheel contents), [24.4](../../DOCUMENTATION.md#244-python); the `changes` structured request (`the_structured_request_reads_the_journal_like_rest`) | publishing on PyPI: the maintainer's decision (package name, token) |
| 30 | modify | Node.js driver | done | 1.3.0: `Date` as `datetime`; `types: "extended"` gives `BigInt` beyond 2^53 and decimals as texts or a class of the application ([24.3](../../DOCUMENTATION.md#243-nodejs)) | publishing on npm: with the release only (`RELEASE.md`) |
| 31 | modify | REST and binary protocol | existing | dates as ISO 8601 in UTC on every interface | |
| 32 | strengthen | Redis compatibility | existing | `tests/redis-clients/check.mjs` (ioredis, node-redis, rate limiters) in CI | |
| 33 | add | latency metrics | done | `kiva_request_duration_seconds` histogram, `GET /ready`, [27.2](../../DOCUMENTATION.md#272-prometheus) | |
| 34 | add | reproducible benchmarks | done | `benches/versions.rs`: real servers of 1.7.1 and 2.0 given the same work through the binary protocol, in rounds taking turns, every answer checked; 2 000, 20 000 and 100 000 documents, 3 rounds each; median, p95, p99, throughput, memory, CPU, disk, compaction, start after a stop and after a crash; the same orders as typed documents and as JSON in 2.0. Run again on `main` at `7c62009`, with the native values: [results and method](benchmarks/versions.md), every round in [`runs`](benchmarks/runs). The planner on its own: [queries.md](benchmarks/queries.md) | one machine (a 2-core laptop, client and server together); no run on Linux, since the shared CI runners are too noisy to publish |
| 35 | modify | Kiva Studio | partial | [fomadev/kiva-studio](https://github.com/fomadev/kiva-studio): projects, grants on scopes, default workspaces, sessions in no workspace, `can_compact`, the types of 2.0 | `doctor` stays a command of the server's machine: Studio connects over the network and cannot read the home |
| 36 | modify | CLI and shell | partial | `kivadb doctor` | |
| 37 | strengthen | replication and failover | existing + done | `tests/failover.rs`; clock drift between nodes (2.0) | |
| 38 | modify | CI and packaging | done | `cargo fmt --check`; Python driver in CI; release candidates as pre-releases (notes from "Unreleased", own Docker tag, never `latest`), the tag checked against `Cargo.toml`; `kivadb update` orders a candidate before its release | |

## Native values (2.0.0)

Decimals, dates, times and datetimes kept as such inside documents, and typed numeric keys ([NATIVE_DOCUMENTS.md](../architecture/NATIVE_DOCUMENTS.md)). Each lot passed the whole test suite and its CI on Windows, Linux and macOS.

| Lot | Item | Status | Proof | Left |
| :- | :- | :- | :- | :- |
| C1 | the `Doc` tree and its binary encoding `KDOC` | done | `src/doc.rs`, unit and property tests, a fuzz target | |
| C2 | typed documents in the engine, storage format 3 raised on need, `rc.2` replicas refused | done | `tests/native.rs` (also against the real `rc.2` and 1.7.1 in CI), backup manifest format 2 | |
| C3 | queries, indexes, sorting, aggregates, schemas and `json incr` on native values, exactly | done | `tests/native_queries.rs` | dates compare as their ISO texts (`CHANGELOG.md`, "Known limits") |
| C4 | typed literals in the shell (`decimal "..."`, `date "..."`, `now()`) | done | `tests/typed_literals.rs`, [10.10](../../DOCUMENTATION.md#1010-typed-values-in-documents) | |
| N2 | `int`, `float` and `decimal` keys; untyped big integers refused | done | `tests/numeric_keys.rs`, [5.2](../../DOCUMENTATION.md#52-number) | |
| C5 | extended JSON for programs (REST, structured requests, binary protocol, changes), Lua typed values | done | `tests/extended_json.rs`, [20.4](../../DOCUMENTATION.md#204-structured-requests-for-tools), [26](../../DOCUMENTATION.md#chapter-26--lua-scripting) | aggregates, `similar`, SSE answer plain JSON |
| C6 | the drivers' `types` option | done | `drivers/node/test/extended.test.js`, `drivers/python/tests/test_extended.py` | |
| C7 | `doctor` names the storage format; `convert <pattern>.<field> to date\|datetime\|time\|decimal [check]`; Chapter 33 | done | `tests/convert_native.rs`, [33](../../DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20) | |
| S | a home of `rc.2`, the path 1.7.1 → `rc.2` → 2.0.0, a crash during `convert`, the examples of the manual run, benchmarks | done | `tests/compat_rc2.rs`, `tests/manual.rs`, [benchmarks/versions.md](benchmarks/versions.md) | published as `v2.0.0-rc.3` |
| R | random values and UUIDs (R1), `shuffle` and `sample` of arrays (R2), `order by random() limit N` (R3), REST, structured requests and drivers (R4) | done | `src/random.rs` (unit and χ² tests), `tests/random_functions.rs` (replication, change journal, backup, queries), `tests/compat_rc3.rs`, [RANDOM_FUNCTIONS.md](../architecture/RANDOM_FUNCTIONS.md) section 12 | published as `v2.0.0-rc.4` |
| rc.5 | collections (alias `workspace`) and projects (B), the layout `projects/<p>/<c>` with nothing moved (C), expire events in the change journal (E), NDJSON export and import (G), metrics by command and the prompt (H), the drivers' `collection` and the parity table (I), exact keys in `get where` (P0) | done | `tests/collections.rs`, `tests/collection_layout.rs`, `tests/expire_events.rs`, `tests/import_export.rs`, `tests/observability.rs`, `tests/driver_parity.rs`, `tests/exact_keys.rs`, `tests/compat_rc4.rs`, [RC5_ARCHITECTURE.md](../architecture/RC5_ARCHITECTURE.md), [benchmarks/rc5.md](benchmarks/rc5.md) | published as `v2.0.0-rc.5`; F, D and J in 2.1 |
| rc.6 | `use <collection>` inside a project enters a collection of the current project, after the names that worked before; `project list` counts collections | done | `tests/collections.rs` | published as `v2.0.0-rc.6` |
| rc.7 | after the audit of the Windows tests of rc.6: the note `also exists` of `use` and the hint of a refusal name only a collection the user may enter; renaming the collection in use says `rename`; tests of a transaction refusing every change of collection | done | `tests/collections.rs` | published as `v2.0.0-rc.7` |
| rc.8 | `use collection <name>` enters the collection of the current project and nothing else; `use project <name>` selects the project even behind a collection of its name; `help` lists the collection commands | done | `tests/collections.rs` | published as `v2.0.0-rc.8` |
| rc.9 | two backups to one file never mix: a temporary file per backup, the archive put in place without replacing a file, its checksum or no archive; backup errors name what failed and the file; the notes of a pre-release hold only its own changes | done | `tests/backup.rs` | published as `v2.0.0-rc.9` |
| rc.10 | test F: crash points in the commit and the recovery (feature `crash-points`), and a damaged record takes its whole transaction away at start | done | `tests/crash.rs`, `tests/txn_recovery.rs` | published as `v2.0.0-rc.10` |
| 2.0.0 | the production readiness audit: a data file damaged after its last clean stop is refused at start and left unchanged, the bytes cut after a crash kept in a copy, `check repair` copies never replaced; concurrent commits stopped by a crash (F13) | done | `tests/txn_recovery.rs`, `tests/crash.rs`, `docs/testing/production-readiness-2.0.0.md` | the release `2.0.0` |
| 2.0.0 | `project current`, `collection current`; `collection create` in the current project, several at once; `collection drop`, `project drop ... cascade`, no file deleted | done | `tests/project_context.rs` | the release `2.0.0` |

## P2 and later

| # | Item | Status | Why |
| :- | :- | :- | :- |
| 40 | `duration` type | deferred | `+ 30 days` and `- 2 hours` already shift moments; a stored duration needs its own text form and arithmetic |
| 41 | time-series engine | not planned | sorted keys make time ranges cheap; a dedicated engine is a product of its own ([ROADMAP.md](../../ROADMAP.md)) |
| 42 | Go and Java drivers | deferred | after the Python driver, from the same protocol |
| 43 | vectors on disk | deferred | product quantisation or DiskANN ([ROADMAP.md](../../ROADMAP.md)) |
| 44 | partitioning | not planned | clustering is a multi-year effort ([ROADMAP.md](../../ROADMAP.md)) |
