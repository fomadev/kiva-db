| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.4 | 143 | 414 | 4658 | 18627 |
| 8 clients, 80% reads | 2.0.0-rc.5 | 139 | 338 | 5421 | 17225 |
| count with group by | 2.0.0-rc.4 | 20341 | 26471 | 27813 | 48 |
| count with group by | 2.0.0-rc.5 | 23547 | 26840 | 29001 | 43 |
| orders, JSON: query | 2.0.0-rc.4 | 4250 | 5566 | 6366 | 220 |
| orders, JSON: query | 2.0.0-rc.5 | 4805 | 7222 | 7635 | 195 |
| orders, JSON: read | 2.0.0-rc.4 | 126 | 191 | 286 | 7376 |
| orders, JSON: read | 2.0.0-rc.5 | 137 | 181 | 231 | 7302 |
| orders, JSON: write | 2.0.0-rc.4 | 147 | 245 | 376 | 5666 |
| orders, JSON: write | 2.0.0-rc.5 | 176 | 277 | 378 | 5127 |
| orders, typed: query | 2.0.0-rc.4 | 4121 | 5255 | 6146 | 227 |
| orders, typed: query | 2.0.0-rc.5 | 5290 | 7326 | 7548 | 176 |
| orders, typed: read | 2.0.0-rc.4 | 116 | 168 | 250 | 7933 |
| orders, typed: read | 2.0.0-rc.5 | 139 | 175 | 239 | 7010 |
| orders, typed: write | 2.0.0-rc.4 | 147 | 232 | 356 | 5233 |
| orders, typed: write | 2.0.0-rc.5 | 150 | 219 | 354 | 4945 |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 4248 | 5374 | 5655 | 222 |
| orders: 10 drawn at random, condition | 2.0.0-rc.5 | 4902 | 7303 | 7345 | 186 |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 583 | 1049 | 1386 | 1587 |
| orders: 10 drawn at random, every order | 2.0.0-rc.5 | 651 | 1239 | 1471 | 1309 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 4539 | 6181 | 6360 | 205 |
| orders: 10 sorted by total, condition | 2.0.0-rc.5 | 5380 | 7356 | 7499 | 179 |
| query an index (100 rows) | 2.0.0-rc.4 | 801 | 1199 | 1386 | 1166 |
| query an index (100 rows) | 2.0.0-rc.5 | 842 | 1216 | 1457 | 1094 |
| query without an index | 2.0.0-rc.4 | 24704 | 31715 | 32539 | 39 |
| query without an index | 2.0.0-rc.5 | 25512 | 31300 | 37494 | 38 |
| read a string | 2.0.0-rc.4 | 110 | 155 | 228 | 8202 |
| read a string | 2.0.0-rc.5 | 124 | 158 | 232 | 7713 |
| transaction of 5 writes | 2.0.0-rc.4 | 861 | 1131 | 1360 | 1136 |
| transaction of 5 writes | 2.0.0-rc.5 | 886 | 1205 | 1333 | 1051 |
| vector search, k = 10 | 2.0.0-rc.4 | 880 | 1207 | 1324 | 1103 |
| vector search, k = 10 | 2.0.0-rc.5 | 728 | 1160 | 1344 | 1164 |
| write a string | 2.0.0-rc.4 | 132 | 185 | 262 | 6927 |
| write a string | 2.0.0-rc.5 | 144 | 206 | 285 | 6525 |
| write an indexed document | 2.0.0-rc.4 | 160 | 228 | 322 | 6025 |
| write an indexed document | 2.0.0-rc.5 | 177 | 245 | 367 | 5325 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1218 | 1659 | 3047 | 764 |
| write with sync_writes (fsync) | 2.0.0-rc.5 | 1269 | 2233 | 2439 | 738 |

| Measure | 2.0.0-rc.4 | 2.0.0-rc.5 |
| :--- | ---: | ---: |
| compaction (ms) | 97.2 | 113.4 |
| data on disk (MB) | 2.8 | 2.8 |
| data on disk after compaction (MB) | 2.8 | 2.8 |
| data on disk before compaction (MB) | 3.1 | 3.1 |
| server CPU time (s) | 2.8 | 3.0 |
| server memory, at the end (MB) | 14.9 | 14.7 |
| server memory, peak (MB) | 15.2 | 15.0 |
| start after a clean stop (ms) | 527.0 | 530.5 |
| start after a crash (ms) | 525.5 | 525.9 |
