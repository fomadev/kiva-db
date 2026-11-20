| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.3 | 156 | 911 | 2854 | 17374 |
| 8 clients, 80% reads | 2.0.0-rc.4 | 142 | 321 | 894 | 11988 |
| count with group by | 2.0.0-rc.3 | 35236 | 37798 | 39365 | 28 |
| count with group by | 2.0.0-rc.4 | 35624 | 93406 | 172147 | 18 |
| orders, JSON: query | 2.0.0-rc.3 | 7139 | 7988 | 8690 | 137 |
| orders, JSON: query | 2.0.0-rc.4 | 7009 | 8200 | 9398 | 142 |
| orders, JSON: read | 2.0.0-rc.3 | 128 | 156 | 187 | 7405 |
| orders, JSON: read | 2.0.0-rc.4 | 141 | 221 | 382 | 6282 |
| orders, JSON: write | 2.0.0-rc.3 | 162 | 225 | 302 | 5235 |
| orders, JSON: write | 2.0.0-rc.4 | 165 | 253 | 370 | 5588 |
| orders, typed: query | 2.0.0-rc.3 | 6510 | 7221 | 8412 | 150 |
| orders, typed: query | 2.0.0-rc.4 | 7190 | 8026 | 8626 | 141 |
| orders, typed: read | 2.0.0-rc.3 | 131 | 186 | 247 | 7190 |
| orders, typed: read | 2.0.0-rc.4 | 142 | 188 | 285 | 6818 |
| orders, typed: write | 2.0.0-rc.3 | 157 | 253 | 345 | 5558 |
| orders, typed: write | 2.0.0-rc.4 | 165 | 281 | 405 | 4670 |
| orders: 10 drawn at random, condition | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 7189 | 7708 | 7741 | 138 |
| orders: 10 drawn at random, every order | 2.0.0-rc.3 | — | — | — | — |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 1074 | 1300 | 1471 | 912 |
| orders: 10 sorted by total, condition | 2.0.0-rc.3 | 6722 | 7532 | 8116 | 150 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 7413 | 8177 | 9015 | 132 |
| query an index (100 rows) | 2.0.0-rc.3 | 963 | 1096 | 1349 | 969 |
| query an index (100 rows) | 2.0.0-rc.4 | 1035 | 1357 | 1863 | 897 |
| query without an index | 2.0.0-rc.3 | 36689 | 37966 | 39196 | 27 |
| query without an index | 2.0.0-rc.4 | 35832 | 41754 | 87798 | 25 |
| read a string | 2.0.0-rc.3 | 120 | 156 | 203 | 7532 |
| read a string | 2.0.0-rc.4 | 122 | 179 | 267 | 7163 |
| transaction of 5 writes | 2.0.0-rc.3 | 964 | 1164 | 1270 | 995 |
| transaction of 5 writes | 2.0.0-rc.4 | 957 | 1348 | 1846 | 975 |
| vector search, k = 10 | 2.0.0-rc.3 | 1021 | 1270 | 1485 | 916 |
| vector search, k = 10 | 2.0.0-rc.4 | 1058 | 1497 | 1850 | 865 |
| write a string | 2.0.0-rc.3 | 138 | 178 | 249 | 6719 |
| write a string | 2.0.0-rc.4 | 137 | 201 | 394 | 6277 |
| write an indexed document | 2.0.0-rc.3 | 165 | 217 | 299 | 5643 |
| write an indexed document | 2.0.0-rc.4 | 165 | 318 | 625 | 5062 |
| write with sync_writes (fsync) | 2.0.0-rc.3 | 2085 | 4665 | 16414 | 252 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 2056 | 14960 | 33541 | 231 |

| Measure | 2.0.0-rc.3 | 2.0.0-rc.4 |
| :--- | ---: | ---: |
| compaction (ms) | 130.3 | 160.3 |
| data on disk (MB) | 2.8 | 2.8 |
| data on disk after compaction (MB) | 2.8 | 2.8 |
| data on disk before compaction (MB) | 3.1 | 3.1 |
| server CPU time (s) | 3.6 | 3.3 |
| server memory, at the end (MB) | 14.7 | 14.8 |
| server memory, peak (MB) | 15.0 | 15.1 |
| start after a clean stop (ms) | 534.9 | 528.2 |
| start after a crash (ms) | 537.2 | 532.6 |
