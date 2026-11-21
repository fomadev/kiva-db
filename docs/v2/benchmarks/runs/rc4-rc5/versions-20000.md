| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.4 | 188 | 478 | 1879 | 21276 |
| 8 clients, 80% reads | 2.0.0-rc.5 | 167 | 406 | 1532 | 22402 |
| count with group by | 2.0.0-rc.4 | 181205 | 199852 | 218775 | 5 |
| count with group by | 2.0.0-rc.5 | 191667 | 199336 | 200178 | 5 |
| orders, JSON: query | 2.0.0-rc.4 | 43363 | 44799 | 48982 | 23 |
| orders, JSON: query | 2.0.0-rc.5 | 43373 | 49261 | 56602 | 22 |
| orders, JSON: read | 2.0.0-rc.4 | 115 | 160 | 228 | 8077 |
| orders, JSON: read | 2.0.0-rc.5 | 115 | 166 | 237 | 8016 |
| orders, JSON: write | 2.0.0-rc.4 | 141 | 216 | 304 | 6458 |
| orders, JSON: write | 2.0.0-rc.5 | 143 | 221 | 336 | 6326 |
| orders, typed: query | 2.0.0-rc.4 | 42897 | 46045 | 47590 | 23 |
| orders, typed: query | 2.0.0-rc.5 | 42749 | 44119 | 46526 | 23 |
| orders, typed: read | 2.0.0-rc.4 | 118 | 166 | 240 | 7922 |
| orders, typed: read | 2.0.0-rc.5 | 117 | 166 | 233 | 8009 |
| orders, typed: write | 2.0.0-rc.4 | 149 | 234 | 335 | 5943 |
| orders, typed: write | 2.0.0-rc.5 | 149 | 242 | 314 | 5951 |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 42973 | 45181 | 45674 | 23 |
| orders: 10 drawn at random, condition | 2.0.0-rc.5 | 42924 | 45440 | 47757 | 23 |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 3345 | 5527 | 5922 | 261 |
| orders: 10 drawn at random, every order | 2.0.0-rc.5 | 3306 | 5028 | 5338 | 273 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 45687 | 46907 | 48906 | 22 |
| orders: 10 sorted by total, condition | 2.0.0-rc.5 | 43968 | 46498 | 46770 | 23 |
| query an index (100 rows) | 2.0.0-rc.4 | 2373 | 3702 | 4234 | 370 |
| query an index (100 rows) | 2.0.0-rc.5 | 2296 | 4107 | 4490 | 364 |
| query without an index | 2.0.0-rc.4 | 211242 | 246235 | 264096 | 5 |
| query without an index | 2.0.0-rc.5 | 224888 | 256954 | 265752 | 4 |
| read a string | 2.0.0-rc.4 | 112 | 159 | 223 | 8225 |
| read a string | 2.0.0-rc.5 | 116 | 164 | 231 | 7969 |
| transaction of 5 writes | 2.0.0-rc.4 | 810 | 1122 | 1500 | 1099 |
| transaction of 5 writes | 2.0.0-rc.5 | 817 | 1146 | 1483 | 1082 |
| vector search, k = 10 | 2.0.0-rc.4 | 817 | 1351 | 1513 | 1111 |
| vector search, k = 10 | 2.0.0-rc.5 | 820 | 1272 | 1594 | 1109 |
| write a string | 2.0.0-rc.4 | 115 | 168 | 249 | 7526 |
| write a string | 2.0.0-rc.5 | 116 | 178 | 249 | 7019 |
| write an indexed document | 2.0.0-rc.4 | 136 | 211 | 306 | 6247 |
| write an indexed document | 2.0.0-rc.5 | 147 | 224 | 322 | 5960 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1137 | 1707 | 2056 | 827 |
| write with sync_writes (fsync) | 2.0.0-rc.5 | 1181 | 1772 | 2101 | 793 |

| Measure | 2.0.0-rc.4 | 2.0.0-rc.5 |
| :--- | ---: | ---: |
| compaction (ms) | 483.9 | 479.6 |
| data on disk (MB) | 19.5 | 19.5 |
| data on disk after compaction (MB) | 19.3 | 19.3 |
| data on disk before compaction (MB) | 22.1 | 22.1 |
| server CPU time (s) | 19.4 | 19.8 |
| server memory, at the end (MB) | 35.5 | 35.2 |
| server memory, peak (MB) | 37.9 | 37.7 |
| start after a clean stop (ms) | 1027.4 | 1023.4 |
| start after a crash (ms) | 1023.7 | 1024.5 |
