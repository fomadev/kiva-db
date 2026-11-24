# KivaDB 2.0.0 — production readiness audit

Audit of `main` after 2.0.0-rc.10 (`ed26356`), the fixes of this audit
included. The crashes inside the commit: [transaction-crash-audit.md](transaction-crash-audit.md)
and [transaction-crash-matrix.md](transaction-crash-matrix.md).

Each criterion is **PASS** (checked, by the test or the check named),
**FAIL** (a defect that stays), or **NOT TESTED** (not checked here: what it
would take is said). A PASS covers what its test covers, and nothing more.

## Decision

**READY for 2.0.0.** The condition set by this audit, the continuous
integration passing on Windows, Linux and macOS, is met (criterion 19). Every blocking criterion
below is PASS on Windows; the defects this audit found (section 1) are fixed,
each with a test that fails on 2.0.0-rc.10. What stays NOT TESTED is a
limit of what a test can prove here (power cut, crash of the operating
system), documented in [17.2](../../DOCUMENTATION.md#172-what-survives-what)
and section 4, or a check left to the owner (manual tests on Windows of the
release built from this branch).

This is not a promise that KivaDB loses nothing on every machine: it
promises what section 4 says, and these tests check it.

## 1. Defects found by this audit, and fixed

The recovery at start took two kinds of damage for a write cut short by a
crash, and deleted what it cut. Reproduced on throwaway homes, 2.0.0-rc.10:

| # | Damage | 2.0.0-rc.10 | Now |
|---|---|---|---|
| P1 | The commit record of the last transaction, whole, one bit changed | The acknowledged transaction left out, its 92 bytes deleted; the start said `incomplete log discarded` | The workspace is not opened; the file is unchanged; the error names byte and damage |
| P2 | One bit of the size field of the first record of a file of 54 keys | The file cut to its 16-byte header: **every key lost**, no copy, an `[INFO]` line | Not opened, file unchanged; `kivadb doctor` FAIL `damaged at byte 16` (it said `a write was interrupted`) |
| P3 | Any cut at start | Bytes deleted; beyond 4 MiB a whole-file copy, over the previous `store.kiva.bak` | The bytes cut are copied first to `store.kiva.cut-<ms>` (created only if absent, synced, folder synced); a start that cannot copy cuts nothing |
| P4 | `check repair` | Its copy replaced the copy of an earlier repair | `store.kiva.bak-<ms>`, never replaced; a compaction that leaves damaged records out keeps one of the whole file first |

The rule chosen with the owner of the release: **a start opens a file only
when what it reads after the last clean stop is whole, or ends in a write cut
short.** Everything else is refused without changing the file
(`ERR_DATA_CORRUPTED: <file> is damaged at byte <n>: <what>. KivaDB did not
open it and changed nothing in it ...`): a whole record whose checksum does
not match, the last one included; an encrypted record that fails its
authentication; a header no record could have; a header whose size runs past
the end of the file although a valid record follows it. A mid-file record
with a wrong checksum was skipped before and is refused now as well. A
damaged collection does not stop the others; a damaged `default` collection
or system database stops the start. Storage format unchanged.

**The limit of the format, documented, not hidden.** It has no marker that
tells a write cut short from damage. A cut write is recognised as: a record
missing bytes with nothing valid after it, or an end of zeros (allotted,
never written, as after a power cut). The one damage that looks the same:
the size field of the **very last** record made larger, nothing valid after
it. It is cut as a crash would leave it, the transaction it ends left out,
and its bytes kept in the copy named at the start
(`a_damaged_last_header_that_looks_cut_is_cut_and_kept`). Removing that
limit needs a format change; not done for 2.0.0.

**Recovery of a refused file.** None is built in: restore a backup, keep the
damaged file. A tool that extracts the verifiable records into a new home,
reading the original only, is left for later; the refused file keeps every
byte for it.

## 2. Criteria

| # | Criterion | Result | Evidence |
|---|---|---|---|
| 1 | Atomicity of a transaction after a crash of the process | PASS | Test F, F1–F12 (`tests/crash.rs`), CI of rc.10 on the 3 systems |
| 2 | No answered transaction lost in silence: damaged commit record at the end (P1) | PASS (fixed) | `a_damaged_commit_record_at_the_end_is_not_taken_for_a_cut_write`; fails on rc.10 |
| 3 | No data cut away on a damaged header in the middle (P2) | PASS (fixed) | `a_damaged_header_in_the_middle_stops_the_start_instead_of_cutting_the_rest`, with `kivadb doctor`; fails on rc.10 |
| 4 | Bytes cut at start kept first, never over another copy (P3) | PASS (fixed) | `a_log_cut_in_the_middle_of_a_transaction_keeps_what_it_cuts`, F11 (a copy per attempt, the same bytes) |
| 5 | `check repair` keeps the original, never over an earlier copy (P4) | PASS (fixed) | `bit_rot_is_detected_and_repaired` (two repairs, two copies, the first equal to the file before) |
| 6 | A refused file is not changed; the error names file, byte and damage | PASS | `refused()` in `tests/txn_recovery.rs` compares the file byte for byte; R1–R3, E, M |
| 7 | A damaged collection does not stop the others | PASS | `a_damaged_collection_does_not_stop_the_others` |
| 8 | A write cut short is still mended without asking | PASS | R4, Z (zeros), F2, F11, F13c, `a_torn_final_write_is_discarded` |
| 9 | Concurrent commits stopped by a crash (F13) | PASS | 4 clients on a server, stopped at the 40th batch after the write, before a group fsync (`sync_writes`), and halfway through a write: exactly 40, 40, 39 transfers, every answered one there; 3 runs |
| 10 | Isolation as documented (13.7): conflicts on the keys written, write skew allowed | PASS | `concurrent_transfers_lose_no_update`, `only_the_keys_written_are_checked_at_commit` |
| 11 | Homes of 1.7.1 open, nothing lost or moved | PASS | `tests/compat.rs` and the fixtures (plain, compressed, encrypted, indexes, TTL, HNSW, accounts), rc.2, rc.3, rc.4 |
| 12 | Backups and restores | PASS | `tests/backup.rs` (two backups at once, never over a file, damaged archive refused) |
| 13 | Indexes, TTL, schemas, queries, typed values | PASS | `tests/queries.rs`, `schemas.rs`, `native*.rs`, `expire_events.rs`, `typed_literals.rs`, `numeric_keys.rs` |
| 14 | Replication and failover | PASS | `tests/failover.rs`, `network.rs` (`replication_and_failover`, `replication_covers_every_workspace`) |
| 15 | Node.js and Python drivers | PASS | Workflows `Node.js driver` and `Python driver`, green on rc.10; `tests/driver_parity.rs` |
| 16 | Authentication, grants, TLS, errors | PASS | `tests/network.rs` (logins), `collections.rs` (grants, refusals that hide data), `tls.rs`. Not audited again line by line for this release |
| 17 | No crash point in the production executables | PASS | rc.10 release: 0 occurrence in the 3 archives, SHA-256 checked; the CI now checks the release executable of each system before archiving it |
| 18 | Full suite on Windows | PASS | 441 passed, 0 failed, 6 ignored; with the feature, `crash` 14 and `txn_recovery` 11 passed; `cargo fmt --check`, `cargo clippy -D warnings` with and without the feature |
| 19 | Full suite on Linux and macOS | PASS | CI of [#154](https://github.com/fomadev/kiva-db/pull/154): 30 checks passed (build and tests on Windows x64, Linux x64, macOS arm64, crash points with the feature, release executables checked, drivers), 4 skipped (release-only steps) |
| 20 | No performance regression | PASS | [stable.md](../benchmarks/stable.md): 20 000 documents, 3 rounds, medians within 3 % but one (+5 %) |
| 21 | Durability against a power cut or a crash of the operating system | NOT TESTED | Section 4: a limit of what these tests prove, documented |
| 22 | Recovery of the verifiable records of a refused file | NOT TESTED | Not built; a backup restores; the refused file keeps every byte |
| 23 | Manual tests on Windows (backup, restore, transactions, a kill) of the build of this branch | NOT TESTED | Left to the owner of the release |
| 24 | Documentation: migration, CHANGELOG, release notes, compatibility | PASS | Chapter 33 (stop cleanly, `check` first), 17.2, 17.4, 19.2, 13.7; CHANGELOG (Upgrading, Fixed, Added); section 3 here |

## 3. Compatibility

| From | Opens in 2.0.0 | Back to it |
|---|---|---|
| Home of 1.7.1 (plain, compressed, encrypted, indexes, unique index, TTL, HNSW, workspaces, accounts) | Yes, accounts migrated at first start, nothing else written | Yes while no workspace holds a `date`/`time`/`datetime` (format 2) or a native document (format 3); accounts read with the rules of 1.x |
| Homes of 2.0.0-rc.2, rc.3, rc.4 | Yes | rc.2 refuses format 3 |
| `.kva` exports of 1.7.1, rc.2, rc.3, rc.4 | Yes | — |
| Replication 1.x ↔ 2.0 | Refused before the snapshot, data kept | — |
| Node.js driver 1.3.0, Python driver 1.1.0 | Against 2.0.0 | — |

Storage formats: 1 (1.x and 2.x), 2 (dates and times, 2.x), 3 (native
values, 2.0.0-rc.3 and later). This audit changes none. The fixtures of
`tests/fixtures` are opened by every run of the suite; the upgrade step by
step is [Chapter 33](../../DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20).

## 4. Durability: what is proved, and what is not

| Event | Promise | Proved by |
|---|---|---|
| Crash or kill of the process | An answered commit is kept; a write cut short is left out whole | Test F: F1–F13, on Windows, Linux, macOS |
| Crash during the recovery | The next start finishes it, same result | F11 |
| Damage on the disk after the last clean stop | Not opened, nothing changed | P1, P2, R1–R3, E, M |
| Power cut, crash of the operating system | Up to one second lost by default; nothing with `sync_writes = true` | **NOT TESTED.** Code read: `sync_data` on the data file before the answer with `sync_writes`, group commit (F13b), folder synced after a rename on Linux and macOS. Proving it needs a machine stopped at random or a file system that drops what was not synced |
| A disk that lies about its fsync, or loses sectors | None can be made | — |

## 5. To run again

```
cargo test
cargo test --features crash-points --test crash --test txn_recovery
cargo clippy --all-targets -- -D warnings
```
