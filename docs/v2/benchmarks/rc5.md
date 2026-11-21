# KivaDB 2.0.0-rc.4 and 2.0.0-rc.5, measured side by side

The candidate `2.0.0-rc.5` adds collections and projects, a new folder layout for new collections, expire events in the change journal, NDJSON export and import, metrics by command and the drivers' `collection` ([RC5_ARCHITECTURE.md](../../architecture/RC5_ARCHITECTURE.md)). These runs check that the work of `rc.4` costs the same in `rc.5`.

The program and the method are those of [versions.md](versions.md) and [rc4.md](rc4.md); every round is in [`runs/rc4-rc5`](runs/rc4-rc5).

## What it says

* **No slowdown that holds from one run to the next.** At 100 000 documents, the benchmark was run twice, 3 rounds each, and the two runs disagree:
  * in the first, `rc.5` is equal or faster on simple operations, 8 to 18 % slower on the queries over the orders, with 3 % less CPU time (104.3 s against 107.1 s);
  * in the second, the queries over the orders are equal (within 4 %), simple operations 7 to 10 % slower, with 10 % more CPU time (121.5 s against 110.9 s).
  * At 20 000 documents, the queries are equal within 1 %. At 2 000 documents, the medians move by up to 28 %, both ways, as they did between `rc.3` and `rc.4`.
* **Nothing in the code between the two versions sits on these paths.** The changes that run on every command or every read are a few counters: a relaxed atomic increment per command and per latency bucket (the metrics by command), one more per query (the strategy chosen), and, for a key found expired, an entry in the change journal when it is on. The planner checks once per query whether the pattern names one key (`KEY LOOKUP`). The engine's reads, writes, indexes, compaction and vector search are those of `rc.4`.
* **Two cases came out slower in both runs:** compaction (+11 % and +11 %: 2.43 s against 2.19 s, 2.68 s against 2.41 s) and the vector search (+10 % and +8 %, under 0.1 ms). Neither code changed. They are given as measured, not explained; a third run on a quiet machine would say whether they hold.
* **Memory and disk are the same:** peak memory within 0.2 MB, data files to the tenth of a megabyte.
* **Start** moves in steps of about 0.5 s in both versions, as in [rc4.md](rc4.md) (Windows refusing a connection to a port not open yet): 3.06 s and 3.07 s in the first run, 3.56 s and 3.05 s in the second.

## Method

* **Versions:**
  * `rc.4`: the tag `v2.0.0-rc.4`, built from source;
  * `rc.5`: `main` at `8f97f44` (every lot of rc.5 merged), built before the version number changed, so that it still calls itself `2.0.0-rc.4`.
  * Both were built in release mode by `rustc 1.99.0`.
* **Cases:** those of [rc4.md](rc4.md), unchanged.
* **Machine:** the machine of [versions.md](versions.md) (Intel Core i5-6300U, 2 cores, Windows 11), client and server on it, nothing else built during the runs.
* **Sizes:** 2 000, 20 000 and 100 000 documents, 3 rounds each, and 100 000 a second time. The first runs took 73 s, 246 s and 1 271 s.

## Results at 100 000 documents (two runs of 3 rounds)

| Case | rc.4, 1st | rc.5, 1st | Change | rc.4, 2nd | rc.5, 2nd | Change |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: |
| write a string | 124 µs | 122 µs | -2 % | 127 µs | 139 µs | +9 % |
| read a string | 124 µs | 119 µs | -4 % | 125 µs | 136 µs | +9 % |
| write an indexed document | 144 µs | 140 µs | -3 % | 146 µs | 161 µs | +10 % |
| transaction of 5 writes | 819 µs | 826 µs | +1 % | 858 µs | 930 µs | +8 % |
| query an index (100 rows) | 2.75 ms | 2.42 ms | -12 % | 2.69 ms | 2.97 ms | +11 % |
| query without an index | 1.06 s | 1.04 s | -2 % | 1.10 s | 1.18 s | +7 % |
| count with group by | 868 ms | 889 ms | +2 % | 951 ms | 1.02 s | +7 % |
| 8 clients, 80% reads | 167 µs | 220 µs | +32 % | 211 µs | 214 µs | +1 % |
| vector search, k = 10 | 851 µs | 937 µs | +10 % | 939 µs | 1.01 ms | +8 % |
| write with sync_writes (fsync) | 1.22 ms | 1.14 ms | -7 % | 1.68 ms | 1.68 ms | 0 % |
| orders, JSON: query | 214 ms | 231 ms | +8 % | 225 ms | 234 ms | +4 % |
| orders, typed: query | 199 ms | 216 ms | +9 % | 227 ms | 230 ms | +1 % |
| orders: 10 sorted by total, condition | 202 ms | 233 ms | +15 % | 226 ms | 225 ms | -1 % |
| orders: 10 drawn at random, condition | 201 ms | 236 ms | +18 % | 224 ms | 224 ms | 0 % |
| orders: 10 drawn at random, every order | 15.9 ms | 17.6 ms | +11 % | 17.5 ms | 18.3 ms | +5 % |

| Measure | rc.4, 1st | rc.5, 1st | rc.4, 2nd | rc.5, 2nd |
| :--- | ---: | ---: | ---: | ---: |
| server CPU time (s) | 107.1 | 104.3 | 110.9 | 121.5 |
| server memory, peak (MB) | 140.0 | 140.1 | 139.1 | 138.9 |
| data on disk after compaction (MB) | 96.4 | 96.4 | 96.4 | 96.4 |
| compaction (ms) | 2 188 | 2 434 | 2 411 | 2 683 |

The tables of 2 000 and 20 000 documents, with p95, p99 and operations per second, are in [`runs/rc4-rc5`](runs/rc4-rc5) (`versions-*.md`; the second run at 100 000 is `versions-100000-second.*`).
