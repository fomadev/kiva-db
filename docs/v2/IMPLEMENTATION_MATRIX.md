# KivaDB 2.0 — implementation matrix

Where each item of the 2.0 plan stands, checked against the code rather than assumed. Every row gives its proof (a file, a test, a section of [DOCUMENTATION.md](../../DOCUMENTATION.md)) and what is left. This page is kept up to date by every pull request of the 2.0 work, and removed or archived once 2.0.0 is released.

Status: **done** (in `main`, tested and documented), **existing** (already in 1.7.1, verified), **partial**, **planned** (in a coming pull request of 2.0), **deferred** (after 2.0, with its reason), **not planned** (rejected in [ROADMAP.md](../../ROADMAP.md), with its reason).

## Baseline

* Version in `Cargo.toml`: 1.7.1 until the release; the work goes to the section "Unreleased (2.0.0)" of [CHANGELOG.md](../../CHANGELOG.md).
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
| 20 | test | compatibility with 1.x | done | `tests/fixtures/v1.7.1*`, made by the 1.7.1 executable from `v1.7.1.kva`; `tests/compat.rs`, which also runs the real 1.7.1 in CI (Linux) to go back | a fixture per later release |
| 39 | document | migration guide | partial | "Upgrading" in the changelog | a migration chapter in DOCUMENTATION.md before the release |

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
| 29 | add | Python driver | done | `drivers/python` (1.0.0, standard library only, typed), `tests/test_driver.py` against a real server, `.github/workflows/python-driver.yml` (Windows, Linux, macOS; Python 3.10, 3.12, 3.14; `mypy --strict`; wheel contents), [24.4](../../DOCUMENTATION.md#244-python); the `changes` structured request (`the_structured_request_reads_the_journal_like_rest`) | publishing on PyPI: the maintainer's decision (package name, token) |
| 30 | modify | Node.js driver | done | 1.2.0, `Date` as `datetime` | |
| 31 | modify | REST and binary protocol | existing | dates as ISO 8601 in UTC on every interface | |
| 32 | strengthen | Redis compatibility | existing | `tests/redis-clients/check.mjs` (ioredis, node-redis, rate limiters) in CI | |
| 33 | add | latency metrics | done | `kiva_request_duration_seconds` histogram, `GET /ready`, [27.2](../../DOCUMENTATION.md#272-prometheus) | |
| 34 | add | reproducible benchmarks | partial | `bench`, `group_commit_speed`, `hnsw_speed` | a suite with documented hardware, comparing 1.7.1 and 2.0 |
| 35 | modify | Kiva Studio | planned | separate repository | projects, grants, dates, `doctor` |
| 36 | modify | CLI and shell | partial | `kivadb doctor` | |
| 37 | strengthen | replication and failover | existing + done | `tests/failover.rs`; clock drift between nodes (2.0) | |
| 38 | modify | CI and packaging | partial | `cargo fmt --check` added; Python driver in CI | |

## P2 and later

| # | Item | Status | Why |
| :- | :- | :- | :- |
| 40 | `duration` type | deferred | `+ 30 days` and `- 2 hours` already shift moments; a stored duration needs its own text form and arithmetic |
| 41 | time-series engine | not planned | sorted keys make time ranges cheap; a dedicated engine is a product of its own ([ROADMAP.md](../../ROADMAP.md)) |
| 42 | Go and Java drivers | deferred | after the Python driver, from the same protocol |
| 43 | vectors on disk | deferred | product quantisation or DiskANN ([ROADMAP.md](../../ROADMAP.md)) |
| 44 | partitioning | not planned | clustering is a multi-year effort ([ROADMAP.md](../../ROADMAP.md)) |
