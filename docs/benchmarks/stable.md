# KivaDB 2.0.0-rc.10 and 2.0.0, measured side by side

The audit of 2.0.0 ([production-readiness-2.0.0.md](../testing/production-readiness-2.0.0.md))
changes the start after a crash (the log read again, damage refused, the
bytes cut kept in a copy), `check repair` and compaction only when they meet
damage. These runs check that nothing else got slower.

The program and the cases are those of [versions.md](versions.md); every
measurement is in [`runs/rc10-stable`](runs/rc10-stable).

## What it says

* **No slowdown.** At 20 000 documents, 3 rounds, every median is within
  3 % of rc.10 except `8 clients, 80% reads` (+5 %, 181 µs against 173 µs,
  its p99 lower), within the spread of this machine
  (two runs of the same versions there differed by up to 18 % during the
  release candidates).
* **The start after a crash**, which reads the log again with the new rule,
  takes the time of rc.10 (1 024 ms against 1 028 ms), most of it the wait
  for the port.
* Memory, disk and CPU time are the same; compaction came out faster
  (504 ms against 687 ms), with no change to its path when nothing is
  damaged: not explained, given as measured.

## Method

* **Versions:**
  * `rc.10`: the executable of the release `v2.0.0-rc.10` (`kivadb_win_x64.zip`,
    its SHA-256 checked against `SHA256SUMS.txt`), built by the continuous
    integration;
  * `2.0.0`: the branch of the audit, built here in release mode by
    `rustc 1.99.0`. The two were not built by the same machine.
* **Machine:** the machine of [versions.md](versions.md) (Intel Core
  i5-6300U, 2 cores, Windows 11), client and server on it.
* **Size:** 20 000 documents, 3 rounds. Runs at 100 000 documents were not
  made for this audit.

## Results

See [runs/rc10-stable/n20000.md](runs/rc10-stable/n20000.md) for the whole
table.

| Case | rc.10 | 2.0.0 | Change |
| :--- | ---: | ---: | ---: |
| write a string | 139 µs | 139 µs | 0 % |
| read a string | 136 µs | 135 µs | -1 % |
| write an indexed document | 165 µs | 163 µs | -1 % |
| transaction of 5 writes | 976 µs | 976 µs | 0 % |
| write with sync_writes (fsync) | 1 692 µs | 1 693 µs | 0 % |
| query an index (100 rows) | 2.45 ms | 2.49 ms | +2 % |
| query without an index | 219 ms | 219 ms | 0 % |
| count with group by | 198 ms | 188 ms | -5 % |
| 8 clients, 80% reads | 173 µs | 181 µs | +5 % |
| vector search, k = 10 | 806 µs | 797 µs | -1 % |
| orders, typed: query | 44.0 ms | 44.9 ms | +2 % |
| start after a clean stop | 1 033 ms | 1 023 ms | -1 % |
| start after a crash | 1 028 ms | 1 024 ms | 0 % |
| compaction | 687 ms | 504 ms | -27 % |
