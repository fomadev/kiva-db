| Case | Version | Median (µs) | p95 (µs) | p99 (µs) | Operations/s |
| :--- | :--- | ---: | ---: | ---: | ---: |
| 8 clients, 80% reads | 2.0.0-rc.4 | 167 | 1138 | 2828 | 19160 |
| 8 clients, 80% reads | 2.0.0-rc.5 | 220 | 570 | 1210 | 19770 |
| count with group by | 2.0.0-rc.4 | 868021 | 1020490 | 1174565 | 1 |
| count with group by | 2.0.0-rc.5 | 888850 | 983309 | 1047326 | 1 |
| orders, JSON: query | 2.0.0-rc.4 | 213674 | 237427 | 269536 | 5 |
| orders, JSON: query | 2.0.0-rc.5 | 230899 | 239485 | 245306 | 4 |
| orders, JSON: read | 2.0.0-rc.4 | 124 | 169 | 223 | 7745 |
| orders, JSON: read | 2.0.0-rc.5 | 124 | 175 | 254 | 7590 |
| orders, JSON: write | 2.0.0-rc.4 | 131 | 185 | 240 | 6572 |
| orders, JSON: write | 2.0.0-rc.5 | 141 | 213 | 326 | 6039 |
| orders, typed: query | 2.0.0-rc.4 | 198783 | 201842 | 203571 | 5 |
| orders, typed: query | 2.0.0-rc.5 | 216038 | 218414 | 219426 | 5 |
| orders, typed: read | 2.0.0-rc.4 | 127 | 165 | 218 | 7544 |
| orders, typed: read | 2.0.0-rc.5 | 126 | 175 | 254 | 7627 |
| orders, typed: write | 2.0.0-rc.4 | 144 | 214 | 287 | 5803 |
| orders, typed: write | 2.0.0-rc.5 | 150 | 233 | 329 | 5321 |
| orders: 10 drawn at random, condition | 2.0.0-rc.4 | 200573 | 261243 | 356246 | 5 |
| orders: 10 drawn at random, condition | 2.0.0-rc.5 | 236223 | 253474 | 263514 | 4 |
| orders: 10 drawn at random, every order | 2.0.0-rc.4 | 15928 | 26738 | 41075 | 52 |
| orders: 10 drawn at random, every order | 2.0.0-rc.5 | 17648 | 24684 | 32405 | 53 |
| orders: 10 sorted by total, condition | 2.0.0-rc.4 | 202156 | 207990 | 208992 | 5 |
| orders: 10 sorted by total, condition | 2.0.0-rc.5 | 232893 | 348381 | 377129 | 4 |
| query an index (100 rows) | 2.0.0-rc.4 | 2746 | 4628 | 5248 | 296 |
| query an index (100 rows) | 2.0.0-rc.5 | 2417 | 3208 | 3972 | 383 |
| query without an index | 2.0.0-rc.4 | 1060666 | 1165050 | 1288958 | 1 |
| query without an index | 2.0.0-rc.5 | 1035964 | 1117065 | 1411245 | 1 |
| read a string | 2.0.0-rc.4 | 124 | 171 | 238 | 7548 |
| read a string | 2.0.0-rc.5 | 119 | 153 | 207 | 8011 |
| transaction of 5 writes | 2.0.0-rc.4 | 819 | 1075 | 1388 | 908 |
| transaction of 5 writes | 2.0.0-rc.5 | 826 | 997 | 1381 | 928 |
| vector search, k = 10 | 2.0.0-rc.4 | 851 | 1102 | 1458 | 1094 |
| vector search, k = 10 | 2.0.0-rc.5 | 937 | 1098 | 1472 | 1020 |
| write a string | 2.0.0-rc.4 | 124 | 166 | 253 | 6714 |
| write a string | 2.0.0-rc.5 | 122 | 152 | 216 | 7075 |
| write an indexed document | 2.0.0-rc.4 | 144 | 225 | 323 | 4812 |
| write an indexed document | 2.0.0-rc.5 | 140 | 197 | 270 | 5534 |
| write with sync_writes (fsync) | 2.0.0-rc.4 | 1224 | 3915 | 4420 | 530 |
| write with sync_writes (fsync) | 2.0.0-rc.5 | 1142 | 1785 | 4031 | 723 |

| Measure | 2.0.0-rc.4 | 2.0.0-rc.5 |
| :--- | ---: | ---: |
| compaction (ms) | 2187.9 | 2434.2 |
| data on disk (MB) | 97.7 | 97.7 |
| data on disk after compaction (MB) | 96.4 | 96.4 |
| data on disk before compaction (MB) | 111.4 | 111.4 |
| server CPU time (s) | 107.1 | 104.3 |
| server memory, at the end (MB) | 118.4 | 118.3 |
| server memory, peak (MB) | 140.0 | 140.1 |
| start after a clean stop (ms) | 3073.1 | 3063.1 |
| start after a crash (ms) | 2556.4 | 3062.5 |
