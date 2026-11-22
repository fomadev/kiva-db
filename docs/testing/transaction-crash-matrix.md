# Test F — matrix of the crash cases

Each case: the state of the files the crash leaves, the states a restart may
find, and the test that checks it. Why these points, and not others:
[transaction-crash-audit.md](transaction-crash-audit.md).

The transaction is the same everywhere: `account:alpha` and `account:beta`
at 100 000, then `begin`, alpha 99 999, beta 100 001, `commit`.

* **before**: alpha 100 000, beta 100 000.
* **after**: alpha 99 999, beta 100 001.
* Any other pair is a broken atomicity, even with a total of 200 000.

Every case also checks, after the restart: the total, `check` with no
damaged record, and a second start that finds the same.

## Crash points (`tests/crash.rs`, feature `crash-points`)

| Case | Point | Setting | Allowed after restart | Why |
|---|---|---|---|---|
| F1 | `batch.before_append` | default | **before** | Nothing of the batch reached the file |
| F2 | `batch.torn_append` | default | **before** | Half a batch, no commit record: left out, the file cut (`incomplete log discarded`) |
| F3 | `batch.after_append` | default | **after** | Every record reached the system before the crash of the process |
| F4 | `batch.mid_index` | default | **after** | The index is in memory only; the file holds the whole batch |
| F5 | `batch.before_sync` | `sync_writes = true` | **after** (crash of the process) | As F3. A power cut here may give **before**: not tested |
| F6 | `batch.after_sync` | `sync_writes = true` | **after** | Synced: also what a power cut would keep |
| F7 | `commit.before_reply` | `sync_writes = true` | **after** | Committed and synced; the client got no answer and must read again |
| F8 | `batch.after_append` | `changes enable` | **after**, and the change journal holds the transfer | The journal is made again from the data at start |
| F9 | `batch.before_append`, 2nd passage | default | the first of two transfers | Aiming at one transaction among several |
| F10 | `batch.after_append`, 5th passage | default | alpha 99 995, beta 100 005 | Five successive commits on the same keys, the fifth stopped |
| F11 | `batch.torn_append`, then `open.before_truncate`, then `open.after_truncate` | default | **before**, then a new transfer goes through | A crash during recovery; recovery done again, idempotent |
| F12 | `batch.before_append` named but not armed | default | **after** | A point fires only when armed |

A case fails when the child process was not stopped, or stopped anywhere
else than the point asked for (its `KIVADB-CRASH-POINT` line on stderr), or
did not end within 60 s, or answered `Transaction committed` before a point
where the client is not answered yet.

## Damaged or cut logs (`tests/txn_recovery.rs`, every build)

The transfer committed, KivaDB stopped and its hint removed (the next start
reads the whole log, as after a crash), then the file changed:

| Case | Change | Allowed | Before the fix |
|---|---|---|---|
| R1 | One bit flipped in the alpha record of the transfer (first of the batch) | **before**, the same at every start | **alpha 100 000, beta 100 001** |
| R2 | One bit flipped in the commit record, with another write after it | **before**; the later write kept | before |
| R3 | A broken batch the file ends in, then a new transfer written | the new transfer | the new transfer |
| R4 | The file cut 3 bytes before its end (in the commit record) | **before**, the same at every start | before |

## Concurrency (`tests/txn_recovery.rs`)

| Case | What | Expected |
|---|---|---|
| C1 | 4 clients, 100 transfers each of 1 from alpha to beta, each reading both balances in its transaction; a conflict is retried | alpha 99 600, beta 100 400: no lost update; conflicts answered `modified by another session` |
| C2 | A transaction rolled back after a write | Nothing of it remains |

## Not covered

* A crash of the operating system, a power cut, a disk that loses sectors
  or lies about its fsync: `abort()` does not stand for them.
* A crash point during concurrent commits of several clients of one server.
* Write skew (a transaction that reads a key and writes another): allowed by
  the documented isolation, so not a failure; not tested as one.
