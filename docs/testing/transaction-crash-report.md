# Test F — report: crashes inside the commit (2.0.0-rc.9)

The audit: [transaction-crash-audit.md](transaction-crash-audit.md). The
cases: [transaction-crash-matrix.md](transaction-crash-matrix.md).

## Verdict

**Atomicity and durability against a crash of the process: PASS** on the
cases tested, after the fix of one defect (a damaged record let part of its
transaction apply, see the audit, section 5).

**Ready for production: NOT READY, for this validation alone.** Not covered:

* a crash of the operating system, a power cut, a disk that loses sectors or
  lies about its fsync (`abort()` proves none of them);
* a crash point during concurrent commits of several clients of one server;
* a damaged commit record at the very end of the file, which is read as a
  torn write: the acknowledged transaction is left out, consistently, with
  no other message than `incomplete log discarded`.

## Crash points (`tests/crash.rs`)

Windows 11, KivaDB built from the branch of this report with
`--features crash-points`; run 5 times in a row, the same results each time.

| Case | Point | Function | Crash | Expected | Found | Invariants | Checksums | Result |
|---|---|---|---|---|---|---|---|---|
| F1 | `batch.before_append` | `Engine::apply_batch_watched` | yes | before | 100000 / 100000 | PASS | PASS | **PASS** |
| F2 | `batch.torn_append` | idem | yes | before | 100000 / 100000, `incomplete log discarded` | PASS | PASS | **PASS** |
| F3 | `batch.after_append` | idem | yes | after | 99999 / 100001 | PASS | PASS | **PASS** |
| F4 | `batch.mid_index` | idem | yes | after | 99999 / 100001 | PASS | PASS | **PASS** |
| F5 | `batch.before_sync` (`sync_writes`) | idem | yes | after (process) | 99999 / 100001 | PASS | PASS | **PASS**, power cut not tested |
| F6 | `batch.after_sync` (`sync_writes`) | idem | yes | after | 99999 / 100001 | PASS | PASS | **PASS** |
| F7 | `commit.before_reply` (`sync_writes`) | `Session::commit` | yes | after, no answer | 99999 / 100001, no `Transaction committed` | PASS | PASS | **PASS** |
| F8 | `batch.after_append`, `changes enable` | `Engine::apply_batch_watched` | yes | after, journal with the transfer | 99999 / 100001, both in `changes read` | PASS | PASS | **PASS** |
| F9 | `batch.before_append`, 2nd passage | idem | yes | first transfer only | 99999 / 100001 | PASS | PASS | **PASS** |
| F10 | `batch.after_append`, 5th passage | idem | yes | fifth transfer | 99995 / 100005 | PASS | PASS | **PASS** |
| F11 | torn write, then `open.before_truncate`, `open.after_truncate` | `Engine::open` | yes, 3 times | before, then a transfer | 100000 / 100000, then 99999 / 100001 | PASS | PASS | **PASS** |
| F12 | named, not armed | — | no (as expected) | after | 99999 / 100001 | PASS | PASS | **PASS** |

"Crash: yes" means the child stopped with an abnormal exit **and** its only
`KIVADB-CRASH-POINT` line names the point asked for. "Invariants": the pair
is before or after as allowed, the total is 200 000, and a second start
finds the same pair. "Checksums": `check` reports no damaged record.

## Damaged or cut logs, concurrency (`tests/txn_recovery.rs`)

| Case | Found | Before the fix | Result |
|---|---|---|---|
| R1 bit flipped in the first record of the transfer | before, at every start; the start warns `1 damaged record(s) skipped, and 1 transaction(s) left out whole` | **alpha 100000, beta 100001** | **PASS** (fails on the previous code) |
| R2 bit flipped in the commit record, a write after it | before; the later write kept | before | **PASS** |
| R3 broken batch at the end, then a new transfer | the new transfer | the new transfer | **PASS** |
| R4 file cut inside the commit record | before, at every start | before | **PASS** |
| C1 4 clients × 100 transfers, conflicts retried | 99600 / 100400 | — | **PASS** |
| C2 rollback after a write | nothing kept | — | **PASS** |

## Other checks

* `cargo fmt --check`; `cargo clippy --all-targets -- -D warnings`, with and
  without `--features crash-points`: clean.
* Full suite without the feature, Windows: **435 passed, 0 failed**,
  including the homes of KivaDB 1.7.1, 2.0.0-rc.2 and rc.4 opened by every
  run.
* The executable built without the feature holds none of the names of the
  crash points (`KIVADB_CRASH_ARMED`: 0 occurrences; with the feature: 1);
  the CI checks it on every pull request.
* Performance: a build without the feature compiles every crash point to
  nothing; the fix adds a few comparisons per record read at start. Not
  measured with a benchmark.
* Linux and macOS: the same two test files run in the CI of the pull
  request, step "Test crash points (test F)".

## To run them again

```
cargo test --features crash-points --test crash --test txn_recovery
```

One case, by hand, on a throwaway folder (`KIVADB_CRASH_*` act only on a
build with the feature):

```
set KIVADB_CRASH_ARMED=1
set KIVADB_CRASH_POINT=batch.after_append
kivadb --home %TEMP%\kivadb-crash -e "begin" -e "set a 1" -e "set b 2" -e "commit"
set KIVADB_CRASH_ARMED=
kivadb --home %TEMP%\kivadb-crash -e "get a" -e "get b" -e "check"
```
