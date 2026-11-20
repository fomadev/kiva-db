| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.3 | 198 | 1454 | 3687 | 14232 |
| 8 clients, 80% reads | 2.0.0-rc.4 | 212 | 509 | 1345 | 16377 |
| count with group by | 2.0.0-rc.3 | 1148611 | 1425642 | 1782806 | 1 |
| count with group by | 2.0.0-rc.4 | 934165 | 1122236 | 1230734 | 1 |
| orders, JSON: query | 2.0.0-rc.3 | 260705 | 345909 | 383617 | 4 |
| orders, JSON: query | 2.0.0-rc.4 | 234450 | 244840 | 246387 | 4 |
| orders, JSON: read | 2.0.0-rc.3 | 151 | 207 | 318 | 6159 |
| orders, JSON: read | 2.0.0-rc.4 | 116 | 157 | 204 | 8193 |
| orders, JSON: write | 2.0.0-rc.3 | 164 | 247 | 494 | 5220 |
| orders, JSON: write | 2.0.0-rc.4 | 121 | 194 | 272 | 6439 |
| orders, typed: query | 2.0.0-rc.3 | 241223 | 263320 | 267700 | 4 |
| orders, typed: query | 2.0.0-rc.4 | 210605 | 221023 | 228171 | 5 |
| orders, typed: read | 2.0.0-rc.3 | 152 | 193 | 249 | 6317 |
| orders, typed: read | 2.0.0-rc.4 | 115 | 161 | 193 | 8172 |
| orders, typed: write | 2.0.0-rc.3 | 171 | 229 | 294 | 5120 |
| orders, typed: write | 2.0.0-rc.4 | 138 | 213 | 292 | 6223 |
| orders: 10 drawn at random, condition | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 223078 | 237729 | 238256 | 4 |
| orders: 10 drawn at random, every order | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 16577 | 25926 | 30362 | 56 |
| orders: 10 sorted by total, condition | 2.0.0-rc.3 | 243818 | 328445 | 374193 | 4 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 222423 | 241564 | 248086 | 4 |
| query an index (100 rows) | 2.0.0-rc.3 | 2906 | 4460 | 6842 | 288 |
| query an index (100 rows) | 2.0.0-rc.4 | 2594 | 3295 | 4567 | 357 |
| query without an index | 2.0.0-rc.3 | 1295461 | 1528004 | 1531965 | 1 |
| query without an index | 2.0.0-rc.4 | 1087747 | 1158640 | 1176692 | 1 |
| read a string | 2.0.0-rc.3 | 106 | 155 | 201 | 8712 |
| read a string | 2.0.0-rc.4 | 108 | 158 | 205 | 8586 |
| transaction of 5 writes | 2.0.0-rc.3 | 1008 | 1295 | 3007 | 732 |
| transaction of 5 writes | 2.0.0-rc.4 | 721 | 1071 | 1512 | 970 |
| vector search, k = 10 | 2.0.0-rc.3 | 1035 | 1851 | 2600 | 821 |
| vector search, k = 10 | 2.0.0-rc.4 | 1006 | 1537 | 1694 | 894 |
| write a string | 2.0.0-rc.3 | 111 | 165 | 230 | 7289 |
| write a string | 2.0.0-rc.4 | 112 | 170 | 229 | 7172 |
| write an indexed document | 2.0.0-rc.3 | 169 | 241 | 398 | 4640 |
| write an indexed document | 2.0.0-rc.4 | 124 | 205 | 296 | 5715 |
| write with sync_writes (fsync) | 2.0.0-rc.3 | 1575 | 2534 | 4436 | 591 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1678 | 11198 | 11767 | 391 |

| Measure | 2.0.0-rc.3 | 2.0.0-rc.4 |
| :--- | ---: | ---: |
| compaction (ms) | 2892.6 | 2899.0 |
| data on disk (MB) | 97.7 | 97.7 |
| data on disk after compaction (MB) | 96.4 | 96.4 |
| data on disk before compaction (MB) | 111.3 | 111.4 |
| server CPU time (s) | 122.4 | 111.9 |
| server memory, at the end (MB) | 118.3 | 118.3 |
| server memory, peak (MB) | 139.7 | 140.0 |
| start after a clean stop (ms) | 3073.0 | 3578.1 |
| start after a crash (ms) | 3569.6 | 4062.1 |
