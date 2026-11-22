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
* a record whose header or body is cut at the end of the file, with no
  valid record after it, or an end of zeros, is a torn write: the file is
  cut there, after a copy of the bytes cut (`store.kiva.cut-<ms>`, written
  and synced first);
* anything else stops the start, which changes nothing in the file
  (`ERR_DATA_CORRUPTED`, with the byte and the damage): a whole record whose
  CRC does not match, the last one included; an encrypted record that fails
  its authentication; a header no record could have, or that does not frame
  its record although a valid record follows it. Since 2.0.0 (see section
  9); before, such a record was skipped and counted.

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

* **The size of the very last record, damaged** so that it runs past the
  end of the file, with nothing valid after it, cannot be told from a write
  cut short: the format has no marker for it. The transaction it ends is
  left out, as after a crash, and its bytes are kept in the copy the start
  names. (Until 2.0.0, a damaged commit record at the end was read the same
  way, and its bytes deleted: see section 9.)
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
* Concurrent commits stopped by a crash point are tested on a server with
  four clients (F13); the points are those of one commit, reached by
  whichever client gets there.

## 9. The production readiness audit of 2.0.0

The recovery of 2.0.0-rc.10 still took two kinds of damage for a write cut
short, and deleted what it cut:

| # | Damage | 2.0.0-rc.10 | 2.0.0 |
|---|---|---|---|
| P1 | The commit record at the end of the file, whole, its CRC wrong | Transaction left out, bytes deleted, `incomplete log discarded` | Not opened, file unchanged |
| P2 | One bit of the size of the first record (54 keys in the file) | File cut to its header: every key lost, no copy | Not opened, file unchanged |
| P3 | Any cut at start | Bytes deleted (a whole-file copy beyond 4 MiB, over the previous one) | Bytes kept in `store.kiva.cut-<ms>` first |
| P4 | `check repair` twice | Second copy over the first (`store.kiva.bak`) | `store.kiva.bak-<ms>`, never replaced |

A mid-file record with a wrong CRC was skipped, and the start went on; that
is now refused too, as the owner of the release chose: applying what follows
damage can apply part of a transaction, and leaving it out loses it without
a word. The rule, and how a cut write is told from damage, is in
`tail_verdict` ([src/engine.rs](../../src/engine.rs)); the results in
[production-readiness-2.0.0.md](production-readiness-2.0.0.md).
