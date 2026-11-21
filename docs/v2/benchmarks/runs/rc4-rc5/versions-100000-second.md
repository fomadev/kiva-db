| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.4 | 211 | 509 | 1099 | 20116 |
| 8 clients, 80% reads | 2.0.0-rc.5 | 214 | 537 | 1246 | 19242 |
| count with group by | 2.0.0-rc.4 | 951474 | 1216001 | 1217791 | 1 |
| count with group by | 2.0.0-rc.5 | 1019023 | 1070872 | 1658021 | 1 |
| orders, JSON: query | 2.0.0-rc.4 | 224573 | 235681 | 236911 | 4 |
| orders, JSON: query | 2.0.0-rc.5 | 233506 | 240423 | 246521 | 4 |
| orders, JSON: read | 2.0.0-rc.4 | 137 | 156 | 174 | 6937 |
| orders, JSON: read | 2.0.0-rc.5 | 131 | 167 | 215 | 7164 |
| orders, JSON: write | 2.0.0-rc.4 | 145 | 173 | 212 | 6170 |
| orders, JSON: write | 2.0.0-rc.5 | 154 | 193 | 247 | 5863 |
| orders, typed: query | 2.0.0-rc.4 | 226832 | 265932 | 312746 | 4 |
| orders, typed: query | 2.0.0-rc.5 | 229611 | 364453 | 382412 | 4 |
| orders, typed: read | 2.0.0-rc.4 | 142 | 158 | 188 | 7003 |
| orders, typed: read | 2.0.0-rc.5 | 137 | 173 | 225 | 6922 |
| orders, typed: write | 2.0.0-rc.4 | 161 | 171 | 226 | 5588 |
| orders, typed: write | 2.0.0-rc.5 | 159 | 197 | 254 | 5561 |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 223688 | 228333 | 229001 | 4 |
| orders: 10 drawn at random, condition | 2.0.0-rc.5 | 223511 | 239755 | 245443 | 4 |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 17468 | 18547 | 19438 | 57 |
| orders: 10 drawn at random, every order | 2.0.0-rc.5 | 18291 | 26341 | 28878 | 52 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 226304 | 227327 | 227702 | 4 |
| orders: 10 sorted by total, condition | 2.0.0-rc.5 | 224889 | 240083 | 251833 | 4 |
| query an index (100 rows) | 2.0.0-rc.4 | 2689 | 4552 | 5115 | 316 |
| query an index (100 rows) | 2.0.0-rc.5 | 2973 | 4331 | 4702 | 288 |
| query without an index | 2.0.0-rc.4 | 1101713 | 1502325 | 1516366 | 1 |
| query without an index | 2.0.0-rc.5 | 1180531 | 1279716 | 1379470 | 1 |
| read a string | 2.0.0-rc.4 | 125 | 150 | 178 | 7751 |
| read a string | 2.0.0-rc.5 | 136 | 164 | 212 | 7142 |
| transaction of 5 writes | 2.0.0-rc.4 | 858 | 1066 | 1283 | 882 |
| transaction of 5 writes | 2.0.0-rc.5 | 930 | 1064 | 1241 | 836 |
| vector search, k = 10 | 2.0.0-rc.4 | 939 | 1111 | 1538 | 1007 |
| vector search, k = 10 | 2.0.0-rc.5 | 1014 | 1491 | 1566 | 852 |
| write a string | 2.0.0-rc.4 | 127 | 153 | 199 | 6803 |
| write a string | 2.0.0-rc.5 | 139 | 161 | 223 | 6290 |
| write an indexed document | 2.0.0-rc.4 | 146 | 200 | 276 | 5256 |
| write an indexed document | 2.0.0-rc.5 | 161 | 207 | 300 | 4982 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1681 | 11747 | 13140 | 369 |
| write with sync_writes (fsync) | 2.0.0-rc.5 | 1680 | 3580 | 4906 | 584 |

| Measure | 2.0.0-rc.4 | 2.0.0-rc.5 |
| :--- | ---: | ---: |
| compaction (ms) | 2410.9 | 2683.1 |
| data on disk (MB) | 97.7 | 97.7 |
| data on disk after compaction (MB) | 96.4 | 96.4 |
| data on disk before compaction (MB) | 111.4 | 111.4 |
| server CPU time (s) | 110.9 | 121.5 |
| server memory, at the end (MB) | 117.3 | 117.3 |
| server memory, peak (MB) | 139.1 | 138.9 |
| start after a clean stop (ms) | 3050.3 | 3555.4 |
| start after a crash (ms) | 3053.9 | 3534.8 |
