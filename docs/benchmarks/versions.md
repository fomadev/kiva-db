# KivaDB 1.7.1 and 2.0, measured side by side

The same work given to a real server of each version, from outside, through the binary protocol: what 2.0 changed in the time of the operations of 1.7.1, and in the memory, the CPU and the disk they take. These are the figures of one machine, measured as described below. They are not a guarantee, and no figure here is given without the run that produced it ([`runs/`](runs)).

The query work of 2.0 (`order by`, `explain analyze`, the planner) is measured on its own in [queries.md](queries.md): 1.7.1 has no `order by`, so it cannot be part of a comparison on equal work. The encoding of typed documents alone is measured in [documents.md](documents.md).

## What it says

The release `2.0.0` is measured against the last candidate, `2.0.0-rc.10`, in [stable.md](stable.md): no slowdown.

These runs measure 2.0 as it is before the candidate `2.0.0-rc.3`, with the native values (`main` at `7c62009`). The earlier runs, made before the native values (`main` at `a15abdb`, 28 October 2026), are kept in [`runs/a15abdb`](runs/a15abdb) and gave the same picture.

* **On the operations of 1.7.1, 2.0 runs at about the same speed, and slightly slower on simple operations with 100 000 documents.**
  * On 2 000 and 20 000 documents, the medians of 2.0 are within 8 % of those of 1.7.1 for most cases, faster as often as slower. At 2 000 documents, the noise of this machine dominates: 8 clients at once come out 16 % faster and vector search 19 % slower, in the same run.
  * On 100 000 documents (3 rounds), 2.0 is 8 to 9 % slower to write and read a string and to write an indexed document, and 5 % slower on transactions. Its CPU time is 7 % higher (99.5 s against 93.2 s). The difference is in every round but one, where both versions are equal. It is small enough to be within what this machine moves from one run to the next (below), and no figure of an earlier run showed it, but it is listed rather than explained away.
  * At 20 000 and 100 000 documents, queries, `count ... group by`, vector search, writes with `sync_writes`, compaction and starting are within 5 %, and 8 clients at once are 12 to 24 % faster with 2.0.
* **Memory and disk are the same:** peak memory within 3 %, data files to within 0.1 MB.
* **Typed documents cost about what JSON documents cost.** The same orders, with a decimal and a datetime (`KDOC`) or a float and a text (JSON), on one server of 2.0:
  * writing a typed order takes 9 to 10 % longer at 2 000 and 100 000 documents, and 4 % less at 20 000;
  * reading one takes the same time within 2 %, except 10 % longer at 2 000;
  * a query that reads every order (`count where total > ...`, an exact decimal comparison for the typed ones) is 2 to 6 % faster on typed documents.
* **One run was disturbed, and was made again.** A first run of 100 000 documents, with 2 rounds, gave 2.0 at 143 µs for a string written against 102 µs in one round, and 103 µs in the other. It was run again with 3 rounds; both runs are in [`runs/`](runs). In the third round of the second run, the last cases of 2.0 (the typed orders) were slowed down by something else on the machine (a query of 1.5 s instead of 0.19 s); the medians leave it out, the round is kept.
* **The tails (p99) are noisier than the medians** on this 2-core laptop, for both versions: a p99 doubles from one round to the next, in either version. They are given, not interpreted.

## Method

* **Program:** [`benches/versions.rs`](../../benches/versions.rs), `cargo bench --bench versions`. It starts each server in a fresh home (`kivadb serve --home ... --no-http --no-resp`), in its default settings, and drives it with the Rust client of the crate (`kivadb::client`), the same for both versions. Every answer is checked: same rows, same counts, the last write found after a crash, the same count from the JSON and the typed orders.
* **Versions:** 1.7.1 is the tag `v1.7.1`, and 2.0 is `main` at `7c62009` with the case of orders added to the benchmark. Both were built in release mode by the same compiler (`rustc 1.99.0`), so that the compiler makes no difference.
* **Rounds:** in each round, each version runs every case once. The versions take turns, and the one that starts alternates from round to round. The tables give the median over the rounds of each statistic; [`runs/`](runs) holds every round.
* **Cases,** for N documents:

  | Case | What it does |
  | :--- | :--- |
  | write a string, read a string | N strings of 100 bytes, written then read in a scattered order, one client |
  | write an indexed document | N documents of about 250 bytes, with two secondary indexes |
  | query an index (100 rows) | 500 structured queries `city = ...`, 100 rows each |
  | query without an index | 20 queries on an unindexed field, every document read |
  | count with group by | 20 `count where age >= 30 group by city` |
  | transaction of 5 writes | `begin`, 5 `set`, `commit`, N/10 times |
  | 8 clients, 80% reads | 8 connections at once, N/2 operations in all |
  | vector search, k = 10 | N/4 vectors of 64 dimensions, an HNSW index, 300 searches |
  | write with sync_writes (fsync) | a server with `storage.sync_writes = true`: N/10 writes, each on disk before its answer |
  | compaction | every string written again, then `compact` |
  | start after a clean stop, after a crash | from starting the process to the first answer, after `shutdown` then after a kill |
  | orders, JSON and typed (2.0 only) | on a server of their own, after the other cases: N/4 orders written by `set` (a float and a text, or `decimal "..."` and `datetime "..."`), read in a scattered order, then 20 `count where total > ...` |

* **Measures:** latency on the client's side, per operation (median, p95, p99), and operations per second over the wall time of the case. The memory (peak and at the end) and the CPU time are those of the server process, read before the compaction; the disk is the data folder.
* **Machine:** Intel Core i5-6300U (2 cores, 4 threads, 2.4 GHz), 11.9 GB of RAM, SSD Micron MTFDKCD512QFM, Windows 11 Pro 10.0.26200, the machine of the earlier runs. Client and server run on that same machine, which leaves them 2 cores to share: the figures are lower than on a server, for both versions alike. No run was made on Linux: the runners of the CI are shared, and their figures move too much to be published as measurements.
* **Sizes:** 2 000, 20 000 and 100 000 documents, 3 rounds each. The run of 100 000 took about 18 minutes for both versions.

## Results

#### 2 000 documents (3 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 174 µs | 146 µs | -16 % | 478 µs | 258 µs | 5.18 ms | 768 µs | 16956 | 10920 |
| count with group by | 21.48 ms | 22.13 ms | +3 % | 30.40 ms | 24.44 ms | 32.43 ms | 25.48 ms | 43 | 44 |
| query an index (100 rows) | 677 µs | 712 µs | +5 % | 1.05 ms | 1.24 ms | 1.17 ms | 1.51 ms | 1007 | 1145 |
| query without an index | 26.88 ms | 24.47 ms | -9 % | 33.34 ms | 26.79 ms | 35.41 ms | 30.09 ms | 36 | 40 |
| read a string | 132 µs | 109 µs | -18 % | 162 µs | 168 µs | 217 µs | 219 µs | 7165 | 8003 |
| transaction of 5 writes | 814 µs | 765 µs | -6 % | 1.12 ms | 1.15 ms | 1.23 ms | 1.39 ms | 1190 | 1160 |
| vector search, k = 10 | 585 µs | 695 µs | +19 % | 939 µs | 1.21 ms | 1.19 ms | 1.33 ms | 1437 | 1247 |
| write a string | 143 µs | 121 µs | -15 % | 192 µs | 187 µs | 257 µs | 251 µs | 6575 | 6863 |
| write an indexed document | 135 µs | 143 µs | +6 % | 223 µs | 244 µs | 293 µs | 305 µs | 6395 | 5951 |
| write with sync_writes (fsync) | 1.52 ms | 1.41 ms | -7 % | 2.07 ms | 2.00 ms | 3.09 ms | 3.27 ms | 623 | 624 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 97.6 | 101.0 |
| data on disk (MB) | 2.8 | 2.8 |
| data on disk after compaction (MB) | 2.8 | 2.8 |
| data on disk before compaction (MB) | 3.1 | 3.1 |
| server CPU time (s) | 2.8 | 2.9 |
| server memory, at the end (MB) | 13.4 | 13.8 |
| server memory, peak (MB) | 13.7 | 14.1 |
| start after a clean stop (ms) | 526.6 | 515.3 |
| start after a crash (ms) | 513.0 | 513.7 |

Orders in 2.0, the same as JSON and as typed documents:

| Operation | Median JSON | Median typed | Change | p99 JSON | p99 typed | Op/s JSON | Op/s typed |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| write | 135 µs | 147 µs | +9 % | 308 µs | 306 µs | 6522 | 5643 |
| read | 114 µs | 125 µs | +10 % | 211 µs | 233 µs | 8012 | 7370 |
| query | 5.21 ms | 5.10 ms | -2 % | 7.29 ms | 6.49 ms | 186 | 191 |

#### 20 000 documents (3 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 205 µs | 155 µs | -24 % | 405 µs | 313 µs | 1.53 ms | 1.44 ms | 23450 | 21919 |
| count with group by | 178.62 ms | 186.82 ms | +5 % | 186.92 ms | 195.17 ms | 189.09 ms | 198.92 ms | 6 | 5 |
| query an index (100 rows) | 2.59 ms | 2.60 ms | +0 % | 2.92 ms | 3.03 ms | 3.89 ms | 4.88 ms | 355 | 360 |
| query without an index | 210.78 ms | 212.91 ms | +1 % | 231.51 ms | 231.19 ms | 234.24 ms | 266.56 ms | 5 | 5 |
| read a string | 98 µs | 97 µs | -1 % | 139 µs | 123 µs | 159 µs | 157 µs | 9407 | 9779 |
| transaction of 5 writes | 655 µs | 682 µs | +4 % | 882 µs | 1.00 ms | 1.13 ms | 1.18 ms | 1316 | 1236 |
| vector search, k = 10 | 761 µs | 799 µs | +5 % | 904 µs | 999 µs | 1.09 ms | 1.39 ms | 1232 | 1150 |
| write a string | 106 µs | 108 µs | +1 % | 144 µs | 155 µs | 179 µs | 184 µs | 8388 | 7308 |
| write an indexed document | 121 µs | 131 µs | +8 % | 177 µs | 194 µs | 218 µs | 241 µs | 7015 | 6045 |
| write with sync_writes (fsync) | 1.35 ms | 1.39 ms | +2 % | 1.97 ms | 1.97 ms | 2.15 ms | 2.13 ms | 715 | 694 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 501.8 | 486.0 |
| data on disk (MB) | 19.5 | 19.5 |
| data on disk after compaction (MB) | 19.2 | 19.3 |
| data on disk before compaction (MB) | 22.1 | 22.1 |
| server CPU time (s) | 18.2 | 18.9 |
| server memory, at the end (MB) | 34.0 | 34.4 |
| server memory, peak (MB) | 36.7 | 36.7 |
| start after a clean stop (ms) | 1026.1 | 1025.4 |
| start after a crash (ms) | 1031.2 | 1024.4 |

Orders in 2.0, the same as JSON and as typed documents:

| Operation | Median JSON | Median typed | Change | p99 JSON | p99 typed | Op/s JSON | Op/s typed |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| write | 132 µs | 127 µs | -4 % | 239 µs | 230 µs | 6723 | 6977 |
| read | 106 µs | 104 µs | -2 % | 160 µs | 163 µs | 9091 | 9207 |
| query | 46.60 ms | 44.49 ms | -5 % | 51.56 ms | 52.58 ms | 21 | 22 |

#### 100 000 documents (3 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 205 µs | 180 µs | -12 % | 451 µs | 379 µs | 858 µs | 985 µs | 22884 | 22602 |
| count with group by | 812.75 ms | 834.76 ms | +3 % | 830.50 ms | 847.14 ms | 830.70 ms | 868.53 ms | 1 | 1 |
| query an index (100 rows) | 2.23 ms | 2.30 ms | +3 % | 2.54 ms | 2.54 ms | 3.60 ms | 3.05 ms | 411 | 401 |
| query without an index | 925.49 ms | 933.85 ms | +1 % | 942.14 ms | 968.73 ms | 1131.46 ms | 971.34 ms | 1 | 1 |
| read a string | 106 µs | 116 µs | +9 % | 141 µs | 154 µs | 167 µs | 184 µs | 9076 | 7996 |
| transaction of 5 writes | 744 µs | 780 µs | +5 % | 876 µs | 936 µs | 1.03 ms | 1.23 ms | 1020 | 978 |
| vector search, k = 10 | 890 µs | 905 µs | +2 % | 1.01 ms | 1.07 ms | 1.37 ms | 1.40 ms | 1081 | 1053 |
| write a string | 108 µs | 116 µs | +8 % | 136 µs | 151 µs | 156 µs | 189 µs | 7957 | 7247 |
| write an indexed document | 125 µs | 137 µs | +9 % | 173 µs | 183 µs | 203 µs | 236 µs | 6088 | 5512 |
| write with sync_writes (fsync) | 1.04 ms | 1.02 ms | -2 % | 2.66 ms | 3.77 ms | 4.20 ms | 4.35 ms | 707 | 708 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 2242.6 | 2073.2 |
| data on disk (MB) | 97.7 | 97.7 |
| data on disk after compaction (MB) | 96.3 | 96.4 |
| data on disk before compaction (MB) | 111.3 | 111.4 |
| server CPU time (s) | 93.2 | 99.5 |
| server memory, at the end (MB) | 116.7 | 117.1 |
| server memory, peak (MB) | 139.6 | 139.2 |
| start after a clean stop (ms) | 2559.0 | 2545.7 |
| start after a crash (ms) | 2550.9 | 2548.7 |

Orders in 2.0, the same as JSON and as typed documents:

| Operation | Median JSON | Median typed | Change | p99 JSON | p99 typed | Op/s JSON | Op/s typed |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| write | 131 µs | 144 µs | +10 % | 195 µs | 215 µs | 6988 | 6231 |
| read | 112 µs | 114 µs | +2 % | 160 µs | 170 µs | 8501 | 8371 |
| query | 198.59 ms | 187.11 ms | -6 % | 202.45 ms | 192.74 ms | 5 | 5 |

## Running it again

```bash
cargo build --release
git worktree add ../kv171 v1.7.1 && (cd ../kv171 && cargo build --release)
KIVADB_BENCH_BINS="1.7.1=../kv171/target/release/kivadb;2.0=target/release/kivadb" \
  KIVADB_BENCH_N=20000 KIVADB_BENCH_ROUNDS=3 cargo bench --bench versions
```

`KIVADB_BENCH_OUT` names the JSON file of the measurements (`bench-versions.json`). On another machine, the figures will differ. What can be compared is the ratio between the versions, measured on the same machine in the same run.
