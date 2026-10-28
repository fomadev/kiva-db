# KivaDB 1.7.1 and 2.0, measured side by side

The same work given to a real server of each version, from outside, through the binary protocol: what 2.0 changed in the time of the operations of 1.7.1, and in the memory, the CPU and the disk they take. These are the figures of one machine, measured as described below. They are not a guarantee, and no figure here is given without the run that produced it ([`runs/`](runs)).

The query work of 2.0 (`order by`, `explain analyze`, the planner) is measured on its own in [queries.md](queries.md): 1.7.1 has no `order by`, so it cannot be part of a comparison on equal work.

## What it says

* **On the operations of 1.7.1, 2.0 runs at about the same speed.** These are the operations both versions have: writes and reads of strings and documents, indexed and unindexed queries, `count ... group by`, transactions, 8 clients at once, writes with `sync_writes`, vector search, compaction and starting.
  * On 2 000 and 20 000 documents, the medians of 2.0 are within 7 % of those of 1.7.1. 2.0 is slightly slower more often than not, by 1 to 7 %, and faster in a few cases, up to 11 %.
  * On 100 000 documents (2 rounds), the medians are within 10 % either way, apart from two cases. The indexed query comes out at +52 %; it was measured again below and is not confirmed. Writes with `sync_writes` come out at −24 %, which is no reason to expect faster fsyncs: the code that syncs is the same in both versions.
  * On this machine, the median of one case moves by up to 20 or 30 % from one round to the next within one version: query without an index on 20 000 documents, 226 to 273 ms for 1.7.1. A difference of a few per cent is within that noise, not a measured change.
* **Memory, CPU and disk are the same:** peak memory within 1 %, data files to within 0.2 MB, the same CPU time to within the noise.
* **One figure was off, and was measured again.** On 100 000 documents, the indexed query (100 rows of 1 000) took 3.5 and 4.2 ms with 2.0 against 2.4 and 2.7 ms with 1.7.1, one figure per round. Measured alone (below), four runs taking turns, it is 2.34 and 2.36 ms for 1.7.1 against 2.38 and 2.38 ms for 2.0: 1 to 2 %. In the benchmark, that query comes right after 200 000 writes, and the large data set ran 2 rounds only. The difference is listed here rather than left out, but the run made to check it does not show it.
* **The tails (p99) are noisier than the medians** on this 2-core laptop, for both versions: a p99 doubles from one round to the next, in either version. They are given, not interpreted.

## Method

* **Program:** [`benches/versions.rs`](../../../benches/versions.rs), `cargo bench --bench versions`. It starts each server in a fresh home (`kivadb serve --home ... --no-http --no-resp`), in its default settings, and drives it with the Rust client of the crate (`kivadb::client`), the same for both versions. Every answer is checked: same rows, same counts, the last write found after a crash.
* **Versions:** 1.7.1 is the tag `v1.7.1`, and 2.0 is `main` at `a15abdb` with the benchmark added. Both were built in release mode by the same compiler (`rustc 1.99.0`), so that the compiler makes no difference.
* **Rounds:** in each round, each version runs every case once. The versions take turns, and the one that starts alternates from round to round. The tables give the median over the rounds of each statistic (the mean of the two for 2 rounds); [`runs/`](runs) holds every round.
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

* **Measures:** latency on the client's side, per operation (median, p95, p99), and operations per second over the wall time of the case. The memory (peak and at the end) and the CPU time are those of the server process, read before the compaction; the disk is the data folder.
* **Machine:** Intel Core i5-6300U (2 cores, 4 threads, 2.4 GHz), 11.9 GB of RAM, SSD Micron MTFDKCD512QFM, Windows 11 Pro 10.0.26200. Client and server run on that same machine, which leaves them 2 cores to share: the figures are lower than on a server, for both versions alike.
* **Sizes:** 2 000, 20 000 and 100 000 documents. The run of 100 000 took 48 minutes for both versions.

## Results

#### 2 000 documents (3 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 145 µs | 146 µs | +0 % | 1.52 ms | 916 µs | 5.35 ms | 5.48 ms | 17542 | 17422 |
| count with group by | 18.87 ms | 19.44 ms | +3 % | 19.88 ms | 20.34 ms | 20.28 ms | 22.57 ms | 52 | 50 |
| query an index (100 rows) | 654 µs | 622 µs | -5 % | 956 µs | 800 µs | 1.20 ms | 1.07 ms | 1390 | 1428 |
| query without an index | 22.24 ms | 22.81 ms | +3 % | 23.53 ms | 24.22 ms | 24.19 ms | 28.66 ms | 45 | 44 |
| read a string | 98 µs | 104 µs | +7 % | 134 µs | 140 µs | 152 µs | 159 µs | 9058 | 8836 |
| transaction of 5 writes | 693 µs | 704 µs | +2 % | 773 µs | 758 µs | 905 µs | 958 µs | 1438 | 1418 |
| vector search, k = 10 | 585 µs | 583 µs | -0 % | 761 µs | 1.09 ms | 852 µs | 1.16 ms | 1577 | 1430 |
| write a string | 111 µs | 112 µs | +1 % | 143 µs | 153 µs | 190 µs | 182 µs | 8332 | 8216 |
| write an indexed document | 128 µs | 128 µs | -0 % | 188 µs | 191 µs | 226 µs | 231 µs | 6935 | 7052 |
| write with sync_writes (fsync) | 1.32 ms | 1.35 ms | +2 % | 1.89 ms | 1.94 ms | 2.71 ms | 2.75 ms | 722 | 680 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 88.9 | 97.9 |
| data on disk (MB) | 2.8 | 2.8 |
| data on disk after compaction (MB) | 2.8 | 2.8 |
| data on disk before compaction (MB) | 3.1 | 3.1 |
| server CPU time (s) | 2.4 | 2.5 |
| server memory, at the end (MB) | 14.3 | 14.5 |
| server memory, peak (MB) | 14.7 | 14.7 |
| start after a clean stop (ms) | 528.5 | 523.7 |
| start after a crash (ms) | 517.6 | 522.2 |

#### 20 000 documents (3 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 167 µs | 169 µs | +1 % | 1.20 ms | 1.27 ms | 2.90 ms | 3.11 ms | 20058 | 19840 |
| count with group by | 197.44 ms | 204.21 ms | +3 % | 216.32 ms | 300.50 ms | 306.99 ms | 325.04 ms | 5 | 5 |
| query an index (100 rows) | 2.52 ms | 2.70 ms | +7 % | 3.78 ms | 4.39 ms | 4.79 ms | 6.64 ms | 362 | 317 |
| query without an index | 241.37 ms | 256.96 ms | +6 % | 248.36 ms | 434.45 ms | 262.76 ms | 540.68 ms | 4 | 3 |
| read a string | 138 µs | 141 µs | +2 % | 152 µs | 183 µs | 178 µs | 252 µs | 7014 | 6664 |
| transaction of 5 writes | 939 µs | 967 µs | +3 % | 1.04 ms | 1.09 ms | 1.18 ms | 1.18 ms | 970 | 888 |
| vector search, k = 10 | 804 µs | 814 µs | +1 % | 1.41 ms | 1.06 ms | 1.52 ms | 1.37 ms | 1045 | 1138 |
| write a string | 131 µs | 116 µs | -11 % | 168 µs | 176 µs | 209 µs | 232 µs | 6596 | 6777 |
| write an indexed document | 166 µs | 166 µs | +0 % | 207 µs | 234 µs | 243 µs | 343 µs | 5088 | 5187 |
| write with sync_writes (fsync) | 1.37 ms | 1.38 ms | +1 % | 1.89 ms | 1.96 ms | 2.18 ms | 2.40 ms | 712 | 697 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 544.9 | 551.7 |
| data on disk (MB) | 19.5 | 19.5 |
| data on disk after compaction (MB) | 19.2 | 19.2 |
| data on disk before compaction (MB) | 22.1 | 22.1 |
| server CPU time (s) | 21.0 | 22.2 |
| server memory, at the end (MB) | 34.9 | 35.1 |
| server memory, peak (MB) | 37.8 | 37.7 |
| start after a clean stop (ms) | 1037.0 | 1036.1 |
| start after a crash (ms) | 1031.5 | 1031.7 |

#### 100 000 documents (2 rounds)

| Case | Median 1.7.1 | Median 2.0 | Change | p95 1.7.1 | p95 2.0 | p99 1.7.1 | p99 2.0 | Op/s 1.7.1 | Op/s 2.0 |
| :--- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 167 µs | 175 µs | +5 % | 1.24 ms | 776 µs | 3.24 ms | 2.00 ms | 17181 | 20835 |
| count with group by | 965.49 ms | 976.79 ms | +1 % | 1207.32 ms | 1266.91 ms | 1318.24 ms | 1510.19 ms | 1 | 1 |
| query an index (100 rows) | 2.55 ms | 3.87 ms | +52 % | 2.85 ms | 5.51 ms | 3.68 ms | 8.73 ms | 353 | 226 |
| query without an index | 1061.06 ms | 1156.05 ms | +9 % | 1361.71 ms | 1388.99 ms | 1434.37 ms | 1600.86 ms | 1 | 1 |
| read a string | 129 µs | 116 µs | -10 % | 170 µs | 158 µs | 210 µs | 197 µs | 7336 | 7971 |
| transaction of 5 writes | 876 µs | 788 µs | -10 % | 1.12 ms | 1.06 ms | 1.53 ms | 1.52 ms | 837 | 928 |
| vector search, k = 10 | 892 µs | 918 µs | +3 % | 1.16 ms | 1.86 ms | 1.49 ms | 3.19 ms | 761 | 628 |
| write a string | 130 µs | 117 µs | -10 % | 167 µs | 168 µs | 223 µs | 278 µs | 6493 | 3803 |
| write an indexed document | 149 µs | 154 µs | +3 % | 197 µs | 252 µs | 256 µs | 495 µs | 5216 | 4303 |
| write with sync_writes (fsync) | 1.44 ms | 1.09 ms | -24 % | 1.91 ms | 1.51 ms | 2.38 ms | 2.19 ms | 661 | 863 |

| Measure | 1.7.1 | 2.0 |
| :--- | ---: | ---: |
| compaction (ms) | 2421.7 | 2256.9 |
| data on disk (MB) | 97.6 | 97.6 |
| data on disk after compaction (MB) | 96.3 | 96.3 |
| data on disk before compaction (MB) | 111.2 | 111.3 |
| server CPU time (s) | 111.6 | 108.2 |
| server memory, at the end (MB) | 117.7 | 118.0 |
| server memory, peak (MB) | 140.4 | 139.5 |
| start after a clean stop (ms) | 3053.6 | 3082.8 |
| start after a crash (ms) | 3067.9 | 2801.3 |

#### The indexed query, measured again

100 000 strings, 100 000 documents with two indexes as in the benchmark, then 300 queries `city = ...` of 100 rows, from the Python driver. Four runs, the versions taking turns:

| Run | Version | Median |
| :--- | :--- | ---: |
| 1 | 1.7.1 | 2.34 ms |
| 2 | 2.0 | 2.38 ms |
| 3 | 1.7.1 | 2.36 ms |
| 4 | 2.0 | 2.38 ms |

`explain analyze` on 2.0 gives what that query does: 1 000 index entries, 100 documents read (from disk: the value cache is off by default), 1.4 to 2.0 ms on the server's side, of which 0.2 to 0.3 ms is planning.

## Running it again

```bash
cargo build --release
git worktree add ../kv171 v1.7.1 && (cd ../kv171 && cargo build --release)
KIVADB_BENCH_BINS="1.7.1=../kv171/target/release/kivadb;2.0=target/release/kivadb" \
  KIVADB_BENCH_N=20000 KIVADB_BENCH_ROUNDS=3 cargo bench --bench versions
```

`KIVADB_BENCH_OUT` names the JSON file of the measurements (`bench-versions.json`). On another machine, the figures will differ. What can be compared is the ratio between the versions, measured on the same machine in the same run.
