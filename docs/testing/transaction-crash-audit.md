# Test F — audit of the transaction commit (2.0.0-rc.9)

What a transaction does on its way to the disk, where a crash can stop it,
and what a restart must find. The cases that check it are listed in
[transaction-crash-matrix.md](transaction-crash-matrix.md), their results in
[transaction-crash-report.md](transaction-crash-report.md).

## 1. What is promised

From [DOCUMENTATION.md](../../DOCUMENTATION.md):

* **13.7** — a committed transaction is written as one batch whose last
  record marks the commit; an incomplete batch is ignored at the next start.
  Leaving with a transaction open is a rollback.
* **13.6** — optimistic concurrency: a commit is refused
  (`ERR_TRANSACTION_CONFLICT`) when a key **it writes** was modified by
  another session since its `begin`. Nothing more is promised: no snapshot
  of the reads, no serializability.
* **17.2** — a crash or kill of the process loses nothing acknowledged; a
  write cut in half is discarded. A power failure or an operating system
  crash loses up to the last second of writes by default, nothing with
  `storage.sync_writes = true`.

## 2. There is no separate log

The data file `store.kiva` is the log: every write is appended to it, and
the index in memory points into it. There is no WAL, no undo, no redo: a
batch is either read back whole at the next start, or not at all.

## 3. The commit, step by step

| # | Where | What happens |
|---|---|---|
| 1 | `Session::begin`, `Session` writes ([src/session.rs](../../src/session.rs)) | The writes of the transaction stay in the session (`Txn.writes`); nothing reaches the engine or the disk. `begin_seq` notes the engine's sequence number. |
| 2 | `Session::commit` → `Engine::apply_batch_watched` ([src/engine.rs](../../src/engine.rs)) | Under the engine lock: conflicts (a key written by someone else after `begin_seq`, a `WATCH`ed key), schemas and their defaults, unique indexes, room for vectors. A refusal here writes nothing. |
| 3 | idem | Each operation becomes a record flagged `FLAG_TXN`; the last one also gets `FLAG_TXN_COMMIT`, and its CRC is computed again. |
| 4 | `Inner::append` | The records go out in **one** `write_all` at the end of the file. If it fails, the file is cut back to its previous length. If the batch needs a newer storage format, the file header is raised and synced first. |
| 5 | `Inner::journal` | With the change journal on (`changes enable`), the batch is written to it, marked as one transaction. |
| 6 | idem | The index in memory takes every key of the batch, still under the lock: no other session sees a part of it. |
| 7 | `Engine::make_durable` → `sync_to` → `run_sync` | With `sync_writes`, the lock is released and the data file is synced (`sync_data`, one fsync shared by the writers waiting at that moment: group commit) **before** `apply_batch_watched` returns. Without it, nothing waits: the server syncs once per second. |
| 8 | `Session::commit` returns | The shell prints `Transaction committed`, the server answers the client. |

**Durable from when:**

* against a crash of the process: from the end of step 4, since the bytes
  are in the operating system's cache, which a dead process does not empty;
* against a power cut or a crash of the system: from the end of step 7, and
  only with `sync_writes = true`.

## 4. Recovery

At start, `Engine::open` reads the hint file (`store.kiva.hint`, written at a
clean stop) and then the log after the part it covers; after a crash the hint
is older or missing, so the end of the log is read again (`scan_log`):

* the records of a batch wait until its commit record shows up, then apply
  together; a batch without it is left out, and the file is cut where it
  started;
* a record whose header or body is cut at the end of the file is a torn
  write: the file is cut there;
* a record whose CRC does not match, in the middle of the file, is skipped
  and counted (`corrupted_detected`).

The change journal is then brought in line with the data file
([22.5](../../DOCUMENTATION.md#225-the-change-journal-changes-kept-read-again-from-a-cursor));
the changes missing from it are made again from the records of the data
file (`Inner::rebuild_changes`), batch by batch with the same rule.

## 5. The defect found, and fixed

**A damaged record inside a transaction let the rest of it apply.** In
`scan_log`, a record with a wrong CRC cleared the records waiting for their
commit, but the records of the same batch that came **after** it were then
taken as a new batch, and applied when the commit record arrived.

Reproduced on a throwaway home: transfer alpha 100000 → 99999 and beta
100000 → 100001, the hint removed as after a crash, one bit flipped in the
alpha record of the transfer. The home opened without an error, with
**alpha = 100000 and beta = 100001** (total 200 001); only `kivadb doctor`
reported `checksum mismatch in the record at byte 109`.

The fix (same storage format): a damaged record that is inside a batch
breaks it, and the records of that batch after it, up to its commit record,
are left out as well. A broken batch the file ends in is cut away with its
bytes, so that a batch written after it later is not taken for its end. The
start says so in its log: `n damaged record(s) skipped, and m transaction(s)
left out whole`. `rebuild_changes` does the same for the change journal.
`tests/txn_recovery.rs` fails on the code before the fix.

## 6. Windows of vulnerability that are not defects

* **A commit record damaged at the very end of the file** is read as a torn
  write: the transaction is left out and the file cut. A transaction that
  was acknowledged disappears, as consistently as if it had never been
  committed, but without a word beyond the usual `incomplete log discarded`.
  Telling a damaged commit record from a write cut short would need a
  marker the format does not have.
* **Reads before the fsync.** With `sync_writes`, another session can read a
  transaction between steps 6 and 7. A power cut there may take away what
  it read; a crash of the process cannot.
* **Isolation.** Only the keys a transaction writes are checked at commit.
  A transaction that reads a key and writes another one is not refused when
  the key it read changed meanwhile (write skew). A transfer writes both
  accounts, so two transfers never lose an update; what is not written is
  not protected.
* **The client of a commit that crashed before its answer** cannot know
  whether it was made: it must read again before retrying.

## 7. Crash points

Places of the commit and of the recovery where a test stops the process
(feature `crash-points`, [src/crash.rs](../../src/crash.rs)). Each one stands
for a state of the files that a crash can leave; none is added without one.

| Point | Function | State of the files when it fires |
|---|---|---|
| `batch.before_append` | `Engine::apply_batch_watched` | Checks done, nothing written |
| `batch.torn_append` | idem | The first record of the batch written, half of the second: a write cut short |
| `batch.after_append` | idem | Every record written, neither the journal of changes nor the index updated |
| `batch.mid_index` | idem | The index updated for the first key, not the second (in memory only) |
| `batch.before_sync` | idem | Lock released, `sync_writes` fsync not run yet |
| `batch.after_sync` | idem | fsync done, `apply_batch_watched` not returned |
| `commit.before_reply` | `Session::commit` | Committed, the client not answered |
| `open.before_truncate` | `Engine::open` | Recovery found where the valid log ends, the file not cut yet |
| `open.after_truncate` | idem | File cut and synced, the start not finished |

A crash point is compiled only with the feature. Without it, `crash_point!`
expands to nothing and `crash::aimed_at` is a constant `false`; the CI checks
that the executable built without the feature holds none of their names.
With it, a point fires only for a process started with
`KIVADB_CRASH_ARMED=1`, `KIVADB_CRASH_POINT=<point>` and, to aim at one
transaction among several, `KIVADB_CRASH_HIT=<n>`. It writes
`KIVADB-CRASH-POINT <point>` on stderr and calls `std::process::abort`: no
unwinding, no `Drop`, no rollback, no flush.

## 8. What these tests do not prove

* `abort()` is a crash of the process. It proves nothing about a crash of
  the operating system, a power cut (the cache of the system and of the disk
  are lost), or a disk that lies about its fsync or loses sectors. Those
  need a virtual machine stopped at random, or a file system that drops what
  was not synced; none is run here.
* A crash during concurrent commits of several clients of one server is not
  tested with crash points; concurrent transfers are tested without crash.
