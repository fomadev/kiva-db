# Documents in KDOC (lot C2)

Measured on 2026-11-01, on one machine: Intel Core i5-6300U (2 cores, 4 threads), 12 GB of memory, Windows 11, SSD, client and server together, other programs running. These numbers hold for that machine only and promise nothing elsewhere.

## The same orders, three ways

`cargo bench --bench documents`, 20 000 orders, each holding a decimal total, a date, a datetime, a time and three lines with decimal prices. JSON and MessagePack hold the decimals and moments as texts. KDOC holds them as native values. Each figure is the median of 7 runs, after 2 warm-up runs.

| Encoding | Bytes per order | Encode | Decode |
| :--- | ---: | ---: | ---: |
| JSON | 290 | 3.03 µs | 24.09 µs |
| MessagePack | 216 | 5.33 µs | 15.12 µs |
| KDOC | 247 | 5.86 µs | 14.33 µs |

An engine writing, reading, reopening and compacting those orders. Reopening rebuilds one index, on `city`. Reads go to the disk because the value cache is off.

| Encoding | Write all | Read all | Reopen | Compact | File after compaction |
| :--- | ---: | ---: | ---: | ---: | ---: |
| JSON | 966 ms | 737 ms | 901 ms | 369 ms | 6.5 MB |
| MessagePack | 676 ms | 596 ms | 567 ms | 334 ms | 5.0 MB |
| KDOC | 1090 ms | 824 ms | 754 ms | 232 ms | 5.6 MB |

What can be said:

- **Size.** KDOC is about 15 % smaller than JSON and 14 % larger than MessagePack for these orders.
- **Encoding and decoding.** Encoding is slower than JSON: the document is checked, then written. Decoding is faster than JSON, close to MessagePack.
- **Engine.** Writing and reading are within the noise of this machine. Nothing says KDOC is faster or slower there.

## JSON documents before and after C2

`cargo bench --bench versions` compares release builds of `main` (9a31376) and of this branch. Both run as real servers and get the same work: 20 000 documents, 3 rounds. All the documents are JSON, so this measures what C2 costs data that never uses native values.

| Case | main median | C2 median | Medians of the 3 rounds, main | Medians of the 3 rounds, C2 |
| :--- | ---: | ---: | :--- | :--- |
| write an indexed document | 175 µs | 165 µs | 162 / 175 / 179 | 165 / 163 / 178 |
| read a string | 133 µs | 136 µs | | |
| write a string | 126 µs | 124 µs | | |
| transaction of 5 writes | 961 µs | 953 µs | | |
| query an index (100 rows) | 4.8 ms | 3.2 ms | | |
| query without an index | 576 ms | 323 ms | 276 / 576 / 771 | 323 / 245 / 571 |
| count with group by | 355 ms | 455 ms | 257 / 355 / 685 | 226 / 455 / 657 |
| write with sync_writes (fsync) | 1.19 ms | 1.49 ms | 1248 / 1190 / 1176 | 1296 / 1487 / 1590 |
| compaction | 1103 ms | 1772 ms | 688 / 1274 / 1103 | 558 / 1772 / 2453 |
| server memory, peak | 36.8 MB | 37.2 MB | | |
| data on disk after compaction | 19.3 MB | 19.3 MB | | |

The rounds of a single version differ by a factor of 2 to 3 on this machine, so no gain or loss can be read from these medians. Two examples:

- `query without an index` is faster in C2, although C2 does not change that path.
- `count with group by` and compaction are slower in C2, but their rounds overlap those of `main`.

The fsync case is slower in all three rounds of C2, by 4 % to 35 %. It is the only case that looks consistent. C2 changes nothing in fsync itself: a write of a JSON document costs one more `match` on the encoding.

These comparisons should be run again on a quiet Linux machine before a candidate (lot S). Disk and memory are the same.
