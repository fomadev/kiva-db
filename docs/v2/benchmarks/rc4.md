# KivaDB 2.0.0-rc.3 and 2.0.0-rc.4, measured side by side

The candidate `2.0.0-rc.4` adds the random functions (R1 to R4 of [RANDOM_FUNCTIONS.md](../../architecture/RANDOM_FUNCTIONS.md)). These runs check two things:

- that the work of `rc.3` costs the same in `rc.4`;
- what `order by random() limit N` costs next to a sorted query.

The program and the method are those of [versions.md](versions.md); every round is in [`runs/rc3-rc4`](runs/rc3-rc4).

## What it says

* **No slowdown from rc.3 to rc.4.**
  * At 2 000 and 20 000 documents, the medians of the two versions are within 10 % of each other in most cases, faster as often as slower: the noise of this machine.
  * At 100 000 documents, `rc.4` is equal on strings (+1 % to write, +2 % to read). It is 11 to 28 % faster on indexed writes, transactions and queries, with 9 % less CPU time (111.9 s against 122.4 s).
  * Nothing in the code between the two versions explains such a gain: the random functions do not touch these paths.
  * The other build of this session shared the machine during part of the runs. The rounds of each version move by as much as these differences (a transaction of 5 writes: 1 000 to 1 017 µs for `rc.3`, 713 to 1 044 µs for `rc.4`). They are read as no slowdown, not as a gain.
* **Memory and disk are the same:** peak memory within 0.3 MB, data files to the tenth of a megabyte.
* **Start:** `rc.4` shows 3.6 s against 3.1 s after a clean stop at 100 000 documents, and the opposite elsewhere. The times of both versions come in steps of about 0.5 s (1.03, 3.07, 3.57, 4.06, 6.10 s): Windows takes about that long to refuse a connection to a port not open yet, and the benchmark waits for the first answer. The difference is one step, not a cost of `rc.4`; the code of a start did not change.
* **The tails (p99) and the writes with `sync_writes`** move from one round to the next in both versions, as in [versions.md](versions.md). They are given, not interpreted.

## Random order

10 rows out of the JSON orders (N/4 of them: 500, 5 000 and 25 000), from the median of 3 rounds:

| Query | 500 orders | 5 000 orders | 25 000 orders |
| :--- | ---: | ---: | ---: |
| `get where oj:*.total > 100.50 order by total limit 10` (rc.3) | 6.7 ms | 47.9 ms | 243.8 ms |
| the same (rc.4) | 7.4 ms | 48.2 ms | 222.4 ms |
| `get where oj:*.total > 100.50 order by random() limit 10` (rc.4) | 7.2 ms | 51.3 ms | 223.1 ms |
| `get where oj:* order by random() limit 10` (rc.4) | 1.1 ms | 5.3 ms | 16.6 ms |

* **With a condition, a random order costs what a sort costs**: every candidate is read and tested, and 10 rows are kept. The reservoir keeps 10 rows, the sort 10 too.
* **On a pattern alone, it reads only the 10 documents drawn**: the draw runs over the keys. It is 13 times faster than the query with a condition at 25 000 orders, and its time grows with the keys looked at, not with the documents.

## Method

* **Versions:**
  * `rc.3`: the tag `v2.0.0-rc.3`, built from source;
  * `rc.4`: the branch `release/2.0.0-rc.4` (`main` at `05c38c0`, with these cases added to the benchmark).
  * Both were built in release mode by `rustc 1.99.0`.
* **Cases:** those of [versions.md](versions.md), plus three on the JSON orders: 10 sorted by their total with a condition, 10 drawn at random with the same condition, and 10 drawn among every order. A version without `order by random()` shows `—`.
* **Machine:** the machine of [versions.md](versions.md) (Intel Core i5-6300U, 2 cores, Windows 11). Client and server ran on it, and another session's build shared it during part of the runs.
* **Sizes:** 2 000, 20 000 and 100 000 documents, 3 rounds each. The runs took 78 s, 287 s and 1 465 s.

## Results at 100 000 documents (3 rounds)

| Case | Median rc.3 | Median rc.4 | Change |
| :--- | ---: | ---: | ---: |
| write a string | 111 µs | 112 µs | +1 % |
| read a string | 106 µs | 108 µs | +2 % |
| write an indexed document | 169 µs | 124 µs | -27 % |
| transaction of 5 writes | 1.01 ms | 721 µs | -28 % |
| query an index (100 rows) | 2.91 ms | 2.59 ms | -11 % |
| query without an index | 1.30 s | 1.09 s | -16 % |
| count with group by | 1.15 s | 934 ms | -19 % |
| 8 clients, 80% reads | 198 µs | 212 µs | +7 % |
| vector search, k = 10 | 1.04 ms | 1.01 ms | -3 % |
| write with sync_writes (fsync) | 1.58 ms | 1.68 ms | +7 % |
| orders, JSON: write | 164 µs | 121 µs | -26 % |
| orders, typed: write | 171 µs | 138 µs | -19 % |
| orders, typed: query | 241 ms | 211 ms | -13 % |

| Measure | rc.3 | rc.4 |
| :--- | ---: | ---: |
| server CPU time (s) | 122.4 | 111.9 |
| server memory, peak (MB) | 139.7 | 140.0 |
| data on disk after compaction (MB) | 96.4 | 96.4 |
| compaction (ms) | 2 893 | 2 899 |

The tables of 2 000 and 20 000 documents, with p95, p99 and operations per second, are in [`runs/rc3-rc4`](runs/rc3-rc4) (`versions-*.md`).
