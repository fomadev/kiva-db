| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.3 | 211 | 514 | 2359 | 20364 |
| 8 clients, 80% reads | 2.0.0-rc.4 | 166 | 391 | 1403 | 22450 |
| count with group by | 2.0.0-rc.3 | 219309 | 354395 | 438771 | 4 |
| count with group by | 2.0.0-rc.4 | 217369 | 346270 | 405277 | 4 |
| orders, JSON: query | 2.0.0-rc.3 | 44709 | 49598 | 52169 | 23 |
| orders, JSON: query | 2.0.0-rc.4 | 50553 | 68800 | 72143 | 19 |
| orders, JSON: read | 2.0.0-rc.3 | 111 | 157 | 196 | 8383 |
| orders, JSON: read | 2.0.0-rc.4 | 113 | 154 | 193 | 8380 |
| orders, JSON: write | 2.0.0-rc.3 | 143 | 215 | 293 | 6145 |
| orders, JSON: write | 2.0.0-rc.4 | 144 | 222 | 364 | 6202 |
| orders, typed: query | 2.0.0-rc.3 | 47316 | 83040 | 90041 | 18 |
| orders, typed: query | 2.0.0-rc.4 | 45450 | 52914 | 53427 | 22 |
| orders, typed: read | 2.0.0-rc.3 | 125 | 184 | 271 | 7392 |
| orders, typed: read | 2.0.0-rc.4 | 114 | 170 | 227 | 8180 |
| orders, typed: write | 2.0.0-rc.3 | 151 | 232 | 298 | 5584 |
| orders, typed: write | 2.0.0-rc.4 | 147 | 228 | 282 | 6054 |
| orders: 10 drawn at random, condition | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 51267 | 60885 | 69058 | 18 |
| orders: 10 drawn at random, every order | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 5320 | 7661 | 8014 | 189 |
| orders: 10 sorted by total, condition | 2.0.0-rc.3 | 47928 | 61312 | 107645 | 21 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 48212 | 58508 | 63346 | 20 |
| query an index (100 rows) | 2.0.0-rc.3 | 2858 | 5213 | 5860 | 260 |
| query an index (100 rows) | 2.0.0-rc.4 | 4164 | 4933 | 5887 | 227 |
| query without an index | 2.0.0-rc.3 | 267143 | 385891 | 576117 | 3 |
| query without an index | 2.0.0-rc.4 | 273422 | 459741 | 469209 | 3 |
| read a string | 2.0.0-rc.3 | 131 | 193 | 314 | 6634 |
| read a string | 2.0.0-rc.4 | 126 | 163 | 297 | 7656 |
| transaction of 5 writes | 2.0.0-rc.3 | 966 | 1201 | 1662 | 920 |
| transaction of 5 writes | 2.0.0-rc.4 | 836 | 1132 | 1582 | 1065 |
| vector search, k = 10 | 2.0.0-rc.3 | 837 | 1299 | 1418 | 1052 |
| vector search, k = 10 | 2.0.0-rc.4 | 861 | 1194 | 1426 | 1014 |
| write a string | 2.0.0-rc.3 | 122 | 181 | 263 | 6918 |
| write a string | 2.0.0-rc.4 | 127 | 195 | 342 | 6497 |
| write an indexed document | 2.0.0-rc.3 | 171 | 275 | 425 | 4676 |
| write an indexed document | 2.0.0-rc.4 | 149 | 217 | 434 | 5697 |
| write with sync_writes (fsync) | 2.0.0-rc.3 | 1462 | 1927 | 2540 | 661 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1232 | 1896 | 4290 | 704 |

| Measure | 2.0.0-rc.3 | 2.0.0-rc.4 |
| :--- | ---: | ---: |
| compaction (ms) | 655.4 | 523.2 |
| data on disk (MB) | 19.5 | 19.5 |
| data on disk after compaction (MB) | 19.3 | 19.3 |
| data on disk before compaction (MB) | 22.1 | 22.2 |
| server CPU time (s) | 24.3 | 22.6 |
| server memory, at the end (MB) | 35.5 | 35.3 |
| server memory, peak (MB) | 38.1 | 38.1 |
| start after a clean stop (ms) | 1039.0 | 1033.9 |
| start after a crash (ms) | 1029.9 | 1038.6 |
