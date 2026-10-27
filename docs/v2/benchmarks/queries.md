# Query planner microbenchmarks

What the query work of 2.0 changed in the time of a query, measured before and after in the same conditions. These are microbenchmarks of single queries on one engine, not the comparison of KivaDB 1.7.1 and 2.0.0 as a whole (which comes with the benchmark suite of item 34 of the [implementation matrix](../IMPLEMENTATION_MATRIX.md)).

## Method

* **Code:** [`benches/queries.rs`](../../../benches/queries.rs), run with `cargo bench --bench queries` (release build, `rustc 1.99.0`).
* **Before:** `main` at `baff8b5`, the same file. That version has no `order by`, so the one function that sorts, `first_in_order`, reads every match with `query_expr` and sorts them in the program, by the instant of the datetimes and the value of the numbers, then keeps the first 20. That is what a program had to do. Every other case calls the same functions in both versions.
* **After:** the branch of this work.
* **Data:** 50 000 documents `k:000000` to `k:049999`, `{"num": i, "grp": i % 100, "x": i % 3, "t": "<datetime>"}`. The datetimes `t` spread over the 12 months of 2026, with four offsets (`Z`, `+01:00`, `-05:00`, `+05:30`). Indexes: B-trees on `num` and `t`, a hash index on `grp`, and a unique index on `(grp, num)`.
* **Engine:** default options without background tasks. The value cache is off (its default), so every document read is read from the data file; the operating system keeps that file in memory.
* **Timing:** each case runs 5 times to warm up, then 40 times. The tables give the median and the 90th percentile in milliseconds, and the queries per second at the median.
* **Runs:** the two programs ran one after the other, three times each, alternating (before, after, before...), on the same machine with nothing else running. The first round of each was slower across the board (the machine warming up) and is left out. The figures below are the medians of rounds 2 and 3, which differ by less than 10% within each version.
* **Machine:** Intel Core i5-6300U (2 cores, 4 threads, 2.4 GHz), 11.9 GB of RAM, SSD (Micron MTFDKCD512QFM), Windows 11 Pro 10.0.26200.

## Results

| Case | Query | Rows | Before (ms) | After (ms) | Change |
| :--- | :--- | ---: | ---: | ---: | :--- |
| A limit, no index | `x = 1 limit 10` | 10 | 7.4 to 8.7 | 1.1 | about 7 times faster |
| `and`, two indexes | `grp = 7 and num < 25000` | 250 | 6.9 to 8.9 | 2.7 to 2.8 | about 3 times faster |
| `and`, a unique index on both | `grp = 7 and num = 707` | 1 | 0.046 | 0.011 to 0.012 | about 4 times faster |
| Order, broad filter | `num >= 0 order by t desc limit 20` | 20 | 571 to 574 | 0.34 | about 1 700 times faster |
| Order, selective filter | `grp = 7 order by t limit 20` | 20 | 5.7 to 5.8 | 5.2 to 5.6 | the same, within the noise |
| Order by the filtered field | `num >= 49000 order by num desc limit 20` | 20 | 11.1 to 12.3 | 0.25 | about 45 times faster |
| One index, one key | `num = 12345` | 1 | 0.009 | 0.009 | the same |
| No index, every match | `x = 1` | 16 667 | 507.6 | 511 to 520 | 1 to 2.5% slower |
| One index, a range | `num < 5000` | 5 000 | 49.3 to 50.0 | 49.6 to 52.0 | 0 to 4% slower |

The raw tables of every round are below.

## What made the difference

* **A limit without an index** used to copy every key of the prefix before reading the first document. Keys are now read 256 at a time, so the query stops after the few it needs. The prefix is still counted to show `Estimated` in the plan of the REST API; the shell's `get where` does not count it.
* **`and`** used to list the keys of every indexed comparison, then intersect the lists: here the 25 000 keys of `num < 25000`. Now only the smallest list is built (the 500 keys of `grp = 7`), and each of its keys is checked against the value that the index of `num` holds for it.
* **A unique index on several fields** used to serve only to refuse duplicates. It now answers an `and` of equalities on all its fields, in one lookup.
* **`order by` with a limit** reads the B-tree index of the order field from its first or last value. Each key is checked against the indexes of the filter, and a document is read only when its row is kept: about the 20 rows asked for, instead of the 50 000 documents read and sorted before. Datetimes are read a day around the limit, because their offsets may reorder them.
* **Order with a selective filter:** reading the index of the order field costs about the same as reading the 500 documents of `grp = 7`. That is because in this data the documents of `grp = 7` fall in 3 months out of 12, so the walk crosses 13 000 entries before finding them. When the filter selects fewer documents, they are read and sorted in memory instead ([decision 7](../DECISIONS.md#7-the-order-of-values-and-the-choice-of-a-plan)).

## Regressions

* **Every match without an index** (1 to 2.5% slower) and **a range on one index** (0 to 4%) run the same plan as before. Reading the keys 256 at a time looks for its place in the key tree again for each batch, and each document now goes through the counters of `explain analyze`. The two versions overlap from one round to the next, so this is at most a few per cent, not a measured certainty.
* No case got slower beyond the noise.

None of these figures is a guarantee: they hold for this data, this machine and these queries. `explain analyze` ([DOCUMENTATION.md 11.5](../../../DOCUMENTATION.md#115-explain--see-how-a-query-will-run)) measures a given query on given data.

## Raw tables

Each table gives the median and the 90th percentile, in ms.

<details>
<summary>Before (main, baff8b5), rounds 1 to 3</summary>

```text
round 1                          median        p90
 limit, no index                 12.344     14.159
 and, two indexes                11.645     13.457
 and, unique index on both        0.095      0.192
 order, broad filter            792.262    861.723
 order, selective filter          8.695     11.492
 order by the filtered field     16.178     20.768
 one index, one key               0.009      0.011
 no index, every match          511.502    559.926
 one index, a range              59.958     75.833
round 2
 limit, no index                  8.697     10.411
 and, two indexes                 8.933     10.263
 and, unique index on both        0.046      0.047
 order, broad filter            571.094    599.928
 order, selective filter          5.657      8.961
 order by the filtered field     12.278     13.970
 one index, one key               0.009      0.009
 no index, every match          507.635    529.443
 one index, a range              49.959     54.422
round 3
 limit, no index                  7.409      9.850
 and, two indexes                 6.871      9.407
 and, unique index on both        0.046      0.052
 order, broad filter            573.767    593.085
 order, selective filter          5.800      8.070
 order by the filtered field     11.089     12.075
 one index, one key               0.009      0.009
 no index, every match          507.627    539.943
 one index, a range              49.325     51.525
```

</details>

<details>
<summary>After, rounds 1 to 3</summary>

```text
round 1                          median        p90
 limit, no index                  1.118      1.760
 and, two indexes                 5.895     16.631
 and, unique index on both        0.012      0.014
 order, broad filter              0.491      1.084
 order, selective filter          5.130      8.413
 order by the filtered field      0.249      0.271
 one index, one key               0.016      0.046
 no index, every match          517.466    537.774
 one index, a range              51.064     56.054
round 2
 limit, no index                  1.090      1.593
 and, two indexes                 2.808      5.116
 and, unique index on both        0.011      0.011
 order, broad filter              0.335      0.463
 order, selective filter          5.153      7.635
 order by the filtered field      0.249      0.313
 one index, one key               0.009      0.009
 no index, every match          511.322    533.296
 one index, a range              52.037     54.086
round 3
 limit, no index                  1.126      1.845
 and, two indexes                 2.710      5.520
 and, unique index on both        0.012      0.012
 order, broad filter              0.342      1.014
 order, selective filter          5.586      8.513
 order by the filtered field      0.249      0.280
 one index, one key               0.009      0.009
 no index, every match          520.354    544.328
 one index, a range              49.596     53.464
```

</details>
