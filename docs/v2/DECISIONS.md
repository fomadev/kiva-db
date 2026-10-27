# KivaDB 2.0 — decisions

The choices of 2.0 that are hard to change later (the on-disk format, the wire formats, compatibility), each with what was considered and why this one was kept.

## 1. Moments are stored as canonical text under new type codes

**Decision.** `date`, `datetime` and `time` get the type codes 8, 9 and 10 (1 to 7 are those of 1.x and keep their meaning). The value is stored as canonical text: `2026-10-27`, `2026-10-27T14:30:00.000Z` (always UTC, always milliseconds), `08:00:00`.

**Considered.** A binary form (days since an epoch, microseconds since an epoch, little-endian integers).

**Why.** Canonical text sorts in chronological order byte by byte, so the B-tree indexes, `range` and the comparisons need no special case; it reads the same on every machine, whatever its time zone; and it goes through compression and encryption like any other value. A binary form would save a few bytes per value and cost a second encoding to maintain.

**The format number.** KivaDB 1.7.1, given a record of an unknown type, takes it for a damaged record: in the middle of the file it skips it (and its next compaction drops it), at the end it cuts it off as an interrupted write. Both lose the value silently. So the first record of a new type raises the file's header from format 1 to format 2, just before the record is written, and 1.x refuses a format 2 file outright. A file without the new types stays at format 1, so that going back to 1.x stays possible for workspaces that do not use them. Compaction keeps the format of the file it replaces; `reset` returns to format 1. The header is synced before the record: a crash in between leaves the new header over records of 1.x only, which 2.x reads, or a cut record that is dropped as any interrupted write.

Tests in `tests/compat.rs`, the first two run against the real 1.7.1 executable when `KIVADB_V1_BIN` names it (the CI downloads it on Linux):

* `a_home_used_by_2_0_without_its_new_types_goes_back_to_1_x`: written, compacted and reopened by 2.0, then read by 1.7.1.
* `a_home_holding_dates_is_refused_by_1_x_and_left_as_it_is`: 1.7.1 refuses it, and its bytes are unchanged.
* `a_crash_while_the_first_date_is_written_loses_nothing_else`: the new header with no record yet, then a date cut in the middle.
* `a_backup_of_a_home_holding_dates_restores_them`: format 2, the dates and the indexes after `restore`.
* `a_file_holding_the_types_of_2_0_is_refused_by_1_x_instead_of_cut`, `a_format_from_a_newer_kivadb_is_refused`.

## 2. A datetime without an offset is read in the session's zone

**Decision.** `set datetime t "2026-10-27 14:30:00"` reads the time in the session's zone (`timezone`, then the account's, then `time.zone`); `...Z` and `...+01:00` are applied. The offset is not kept: two writings of the same instant are equal.

**Considered.** Refusing a datetime without an offset, as a strict ISO 8601 instant would.

**Why.** People type local times in a shell, and every server has a zone (`time.zone`, the machine's by default), so the reading is never ambiguous. Programs receive ISO 8601 in UTC on every interface (REST, structured requests, Redis, drivers), so nothing depends on the zone once the value is stored.

## 3. No `kivadb upgrade` command

**Decision.** 2.0 opens the homes of 1.x as they are. The only change made to existing data is to the accounts, at the first start: each personal workspace `u_<name>` becomes a write grant and the account's default workspace.

**Considered.** `kivadb upgrade --check` and `kivadb upgrade`, converting the files into a new folder before switching.

**Why.** The data files, index definitions, hint files and HNSW graphs of 1.7.1 are read unchanged (format 1; only a file that receives a value of the new types moves to format 2, see 1). A command that converts nothing would only add a step to forget. What an upgrade needs is a backup (`kivadb backup`) and a check (`kivadb doctor`), both already there. `tests/compat.rs` proves the reading on homes written by the 1.7.1 executable. Going back to 1.x is possible only for data without the new types; the changelog says so.

## 4. Readiness is not liveness

**Decision.** `GET /health` answers as long as the process runs; `GET /ready` answers `503` while it stops, and on a replica not connected to its master.

**Why.** A load balancer or Kubernetes must not send reads to a replica that is still receiving its first snapshot (old data), but must not restart it either: it is healthy, only not ready.

## 5. The change journal is derived from the data file

**Decision.** The change journal (`store.kiva.changes/`) is written under the engine lock right after the data, each change recording where its data record ends. When a workspace opens, the journal is brought in line with the data file: changes whose data a crash lost are dropped, changes the journal lacks are rebuilt from the data records past the last one it names. Reads sync the data file and the journal before handing out changes. A compaction syncs the journal and records the new data file before it replaces the old one.

**Considered.** Making the journal the source (write it first, then the data), or syncing both files at every write.

**Why.** Without an fsync between the two files, the operating system may write either one first: a journal written first could hold changes whose data never reached the disk, or lose changes whose data did. The data file is already the log the engine trusts after a crash (checksums, transaction commit records), so deriving the journal from it closes the window without a second fsync per write. Handing out only what is synced keeps readers from seeing a change a crash would take back. Rebuilt changes keep their numbers; whether a rebuilt key was created is not known, and is said so (`"created": null`).

**Delivery.** At least once: a reader may get a change again (a rebuild, a cursor read again), so applying one must be idempotent. Exactly once would need the reader's own writes in the same transaction as its cursor, which KivaDB cannot offer across systems.

**Not covered.** Cursors are per node (each journal numbers its own changes), expirations are not changes, and backups do not carry the journal. Those are documented limits, not simulated guarantees.

## 6. Schemas and journals are defined per node, for now

Like secondary indexes, schemas and change journals are set on each node. A replica applies what its master accepted (validated, with its defaults filled in), so validation never diverges on the data that flows; what differs after a failover is what the new master checks next. Replicating these definitions (with the index definitions) is a later work item: it needs the definitions in the replication stream and a rule for a replica whose own definitions differ.
