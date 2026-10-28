# Changelog

Every version of KivaDB, newest first. Version numbers follow [Semantic Versioning](https://semver.org): MAJOR for incompatible changes, MINOR for new features, PATCH for compatible improvements and fixes.

## Unreleased (2.0.0)

Accounts reach only what they are granted, and projects group the workspaces of one application under its name (`travel.places`, `school.places`). Dates, times and datetimes are types of their own, with time zones.

### Upgrading

* **Incompatible:** a new account no longer gets a personal workspace `u_<name>`. It reaches no workspace until an administrator grants it some, and lands in none at login unless it has a default workspace or can reach only one. Scripts that create a user and then write with it must give it access (`create user ... grant write on <workspace>`).
* Existing accounts keep working: on its first start, 2.0 turns each personal workspace `u_<name>` into a write grant and makes it the account's default workspace, so applications connected with those accounts land on their data as before. Accounts saved by 2.0 carry the new model; an earlier version reads them, but gives back to each user a `u_<name>` workspace reached without a grant, and ignores the grants on scopes such as `travel.*`. Make a backup before upgrading if you may need to go back.
* `compact` outside the admin role now needs write access to every workspace of the project (`grant write on travel.*`), instead of the personal workspace.
* The first `date`, `datetime` or `time` written to a workspace raises its data file to storage format 2. KivaDB 1.x refuses such a workspace (`unsupported storage format 2`) rather than taking those records for a damaged end of log and cutting them off; a workspace without them stays readable by 1.x. Make a backup before upgrading (`kivadb backup`) if you may need to go back ([19.4](DOCUMENTATION.md#194-the-data-file-byte-by-byte)).
* Account creation dates (`users`) are shown in the server's time zone (`time.zone`, the machine's by default, as before), not always the machine's.
* The `users` structured request answers `default_workspace` instead of `home`; `whoami` answers `workspace` and `access` as `null` for a session in no workspace.

### Added

* Projects: `project create travel`, then `workspace create travel.places`. Two projects may each have a workspace of the same name. `project list`, `workspace list travel`, and `project remove` for an empty project ([14.6](DOCUMENTATION.md#146-projects)).
* Grants on scopes, several at once: `grant write on travel.*, school.places to "app"`, `grant read on *.places`, `grant write on *.*`. A grant on `travel.*` covers the workspaces created in `travel` later. An editor with write on `travel.*` manages the project: creates its workspaces and compacts them ([16.4](DOCUMENTATION.md#164-workspaces-and-grants)).
* `create user "app" identified by "..." grant write on travel.* default workspace travel.places`: the account and its access in one command. `modify user "app" default workspace travel.places` chooses where it lands at login ([16.3](DOCUMENTATION.md#163-managing-accounts), [16.5](DOCUMENTATION.md#165-logging-in)).
* Log in straight into a workspace as `<user>@<workspace>`, in the shell, over HTTP and from Redis clients (`redis://app%40travel.places:secret@host:6379`), which send it again at every reconnection. `SELECT travel.places` opens a workspace by its name ([23.4](DOCUMENTATION.md#234-logins-databases-roles)).
* `workspace move places_travel to travel.places` renames a workspace or moves it into a project; its files stay where they are, and its grants follow it ([14.7](DOCUMENTATION.md#147-renaming-or-moving-a-workspace)).
* Three types for moments: `set datetime created_at "2026-10-27 14:30:00"` (an instant, in UTC to the millisecond), `set date born "2004-05-09"`, `set time opens "08:00"`. `now()` (or `current_timestamp`), `today()` (`current_date`) and `current_time` give the moment of the action, moved by `+ 30 days`, `- 2 hours`... A text is never turned into a date by itself ([5.8](DOCUMENTATION.md#58-date-datetime-time)).
* `year()`, `month()`, `day()`, `hour()`, `minute()`, `second()` and `weekday()` in `print`, in conditions (`get where month(order:*.created_at) = 10`) and in `group by`. Conditions compare the ISO dates of documents as moments when the value is one (`created_at > now() - 7 days`), whatever their form; a B-tree index answers them, and `year(x) = 2026` too ([11.1](DOCUMENTATION.md#111-the-condition)). `json set k .seen now()` writes ISO 8601 in UTC.
* Time zones: `config set time.zone Africa/Kinshasa` for the server, `modify user "app" timezone ...` for an account, `timezone ...` for a session. Datetimes are kept in UTC and shown in the zone; programs (REST, structured requests, Redis, drivers) always get ISO 8601 in UTC ([5.9](DOCUMENTATION.md#59-time-zones)).
* Replicas compare their clock with the master's at every heartbeat: `stats` shows the difference, and beyond one second the replica warns in its log and the audit log, so that NTP gets turned on ([25.4](DOCUMENTATION.md#254-watching-it)). `stats` also shows the server's time in UTC and in its zone.
* Node.js driver 1.2.0: a `Date` is stored as a `datetime` and comes back as a `Date`; `date` and `time` through `{ type }` ([24.3](DOCUMENTATION.md#243-nodejs)).
<<<<<<< Updated upstream
* `kivadb doctor` checks a home without changing it, whether KivaDB runs or not: the folder and the free disk space, the ports, access open to other machines without passwords, the TLS certificate, the encryption key, the time zone, the replication settings, the checksum of every record of every workspace, and the errors in `server.log`. Each finding says what to do; `--json` gives them to programs, and the exit code is 1 when one is a problem ([17.9](DOCUMENTATION.md#179-kivadb-doctor--a-health-check-of-the-home)).
* `GET /ready`, a readiness probe for load balancers and Kubernetes: `503` while KivaDB stops, or on a replica not connected to its master ([27.1](DOCUMENTATION.md#271-quick-looks)).
* `kiva_request_duration_seconds` is a Prometheus histogram (buckets from 0.1 ms to 5 s), so that `histogram_quantile()` gives the 95th or 99th percentile of command latency; `_sum` and `_count` are unchanged ([27.2](DOCUMENTATION.md#272-prometheus)).
* Homes written by KivaDB 1.7.1 (plain, compressed, encrypted, with indexes, a unique index, expiry, an HNSW graph, workspaces and accounts) are kept in the repository and opened by every test run, so that a change that would stop 2.0 from reading them fails before it is merged. The continuous integration also checks the formatting (`cargo fmt --check`).
=======
* An official Python driver, `kivadb` 1.0.0 ([`drivers/python`](drivers/python)): the binary protocol, the standard library only, typed. `datetime` (with a zone, a naive one is refused), `date` and `time` map to KivaDB's types; CRUD, `query` with `order_by` and `after`, `explain(analyze=True)`, aggregates, vector search, the change journal, `with db.transaction():`, notifications, TLS with a pinned fingerprint, a timeout and reconnection that never sends a request twice. Tested against a real server on Windows, Linux and macOS with Python 3.10 to 3.14 and `mypy --strict`; not published on PyPI yet ([24.4](DOCUMENTATION.md#244-python)).
* The `changes` structured request reads the change journal like `GET /v1/changes`, from the binary protocol and from Redis clients (`KIVA.API`).
* `get where ... order by <field> [asc|desc] limit N`: rows sorted by the value of a field (datetimes with an offset by their instant, times as times, documents without the field last), and `after <key>` for the next page, without a gap or a repeat. `reverse` gives key order from the last key; it used to be accepted and ignored. Over REST, `"order_by": "created_at desc"`, `"after"`, `"reverse"`. With a B-tree index on the field, a limited query reads it in order and stops ([11.2](DOCUMENTATION.md#112-get-where--find-documents)).
* `explain analyze` runs a query and shows what it did: index entries, keys examined, documents read (from the value cache or the disk), filtered out, sorted and returned, and the time of planning, reading and sorting. Over REST, `{"op": "explain", ..., "analyze": true}`. `explain` alone still reads no document; neither runs a write ([11.5](DOCUMENTATION.md#115-explain--see-how-a-query-will-run)).
* Faster queries, the answers unchanged: an `and` lists the keys of its smallest indexed part and checks the others against their indexes instead of listing theirs; a unique index on several fields answers an `and` of equalities on them; a limited scan stops without copying every key of the pattern. Measured on 50 000 documents: 3 to 7 times faster for those, and about 1 700 times for `order by ... limit 20` with a broad filter compared with sorting in the program ([docs/v2/benchmarks/queries.md](docs/v2/benchmarks/queries.md)).
* A durable change journal (change data capture): `changes enable` keeps every committed write of a workspace on disk, numbered, for a retention by age and size (`retain 24h max 256mb`, `keys only` to keep no values). `changes read` and `GET /v1/changes` (with `wait` for a long poll) give the changes after an opaque cursor, in commit order, transactions marked whole; rolled-back transactions give none. A change is handed out only once its data and itself are on disk, and after a crash the journal is brought in line with the data file (changes rebuilt or dropped), so data and changes always agree. Named consumers resume where they acknowledged, at least once; expired and foreign cursors are refused (`CURSOR_EXPIRED`, `CURSOR_INVALID`). Reading needs read access, checked at every read; the journal is encrypted with the workspace. Prometheus shows its size and each consumer's lag ([22.5](DOCUMENTATION.md#225-the-change-journal-changes-kept-read-again-from-a-cursor)).
* Optional schemas: `schema set user: {"email": {"type": "string", "required": true, "unique": true}, "created_at": {"type": "datetime", "default": "now()"}}` gives the documents of a pattern rules and defaults. Rules: `type` (the JSON types, `integer`, `date`, `datetime`, `time`), `required`, `nullable`, `unique` (through a unique index), `min`/`max`, `minLength`/`maxLength`, `pattern` (a regular expression run in linear time), `enum`, `minItems`/`maxItems`/`items` for arrays and `properties` for nested objects. Defaults are values, or `now()`, `today()` and `current_time`, computed at the write (at the commit for a transaction, one moment for all of it). Every write path checks them, the whole document after a partial change, and a refused write changes nothing; the error starts with `SCHEMA:` (REST: 422). Patterns without a schema, and the documents stored before one, are left as they are; `schema check` lists those that do not follow it ([11.8](DOCUMENTATION.md#118-schemas-rules-for-the-documents-of-a-pattern)).
>>>>>>> Stashed changes

## 1.7.1 - 2026-10-27

A fix release: command files saved on Windows run as they are.

### Fixed

* `kivadb run`, `source` in the shell and `script load` refused a file saved by Windows PowerShell 5.1 (`>`, `Out-File`, `Set-Content -Encoding UTF8`): its first line began with a byte order mark (`Unknown command: 'set'`, with an invisible mark before `set`), or the whole file was UTF-16. They now read UTF-8 with or without a byte order mark, and UTF-16 with one; other bytes are refused before anything runs, with a message that says so ([3.10](DOCUMENTATION.md#310-command-files-kva)).

## 1.7.0 - 2026-10-27

Queries with `in`, `like` and `ilike`, unique indexes (on several fields, optionally ignoring case), keys named by KivaDB with `add`, and a guide for applications coming from MongoDB.

### Upgrading

* An earlier version does not know the unique indexes: it does not check them, and it may forget them when it saves its index definitions. If you go back to 1.6, check `indexes` after upgrading again and create them anew.

### Added

* Conditions take `in`, `like` and `ilike`: `city in ["Goma", "Bukavu"]` (one of up to 1 024 values), `name like "Kin%"` (`%` for any text, `_` for one character, `\%` and `\_` for themselves) and `name ilike "kin%"`, which ignores capitals in every alphabet. `not in`, `not like` and `not ilike` negate them. They work in `get where`, the aggregates, `explain`, vector search filters and the structured requests (`"cmp": "in"`, or `{"city": ["Goma", "Bukavu"]}` in `where`). An index answers them: `in` looks up each value, `like` with text before its first wildcard reads only the B-tree values starting with it, and the other patterns are tested on the index's distinct values without reading the documents ([11.1](DOCUMENTATION.md#111-the-condition), [11.4](DOCUMENTATION.md#114-indexes--making-queries-fast)).
* Unique indexes: `create unique index on user: (email)` refuses a second document of the pattern holding the same value, and `create unique index on place: (type, parent, name)` the same combination of values. With `ignore case`, texts are compared without regard to capitals, in every alphabet (`Goma` and `GOMA` conflict), and the documents keep their own text. The check is made under the same lock as every write (shell, structured requests, REST, Redis clients including `HSET` and `JSON.*`, scripts), and a transaction is judged by what its keys hold at `commit`; in `MULTI`, `EXEC` refuses the whole transaction. `null` is a value like the others; a document lacking one of the fields is not constrained. A refused write answers an error starting with `UNIQUE:` (shell, binary protocol, REST with code 409) or `UNIQUE` (Redis clients). Creating the index is refused while two documents already share a value, naming them ([11.7](DOCUMENTATION.md#117-unique-indexes), [23.14](DOCUMENTATION.md#2314-unique-indexes-and-add-from-a-redis-client)).
* `add place: {...}` creates a key named by KivaDB: the prefix followed by a new 24-character identifier in the format of MongoDB's ObjectId (time, random part per process, counter), so keys added later sort after the earlier ones. The structured request `add` answers the key and the identifier ([6.7](DOCUMENTATION.md#67-add--let-kivadb-name-the-key)).
* Node.js driver 1.1.0: `add(prefix, value)` and `query(pattern, where)`, and `err.code` gives the code word of KivaDB's message (`UNIQUE`, `READONLY`...) ([24.3](DOCUMENTATION.md#243-nodejs)).
* [Coming from MongoDB](DOCUMENTATION.md#249-coming-from-mongodb): the counterpart of each operation, and how to copy a collection exported with `mongoexport`, keeping its identifiers.

### Changed

* `in`, `like` and `ilike` are no longer refused: until 1.6.1 they answered that the comparison operators were the six of `= != > >= < <=`. The table of [11.6](DOCUMENTATION.md#116-what-a-query-does-not-do) lists what is still refused (`between`, `contains`, `matches`, `regexp`), and a structured condition may hold a list as value, for `in`.

## 1.6.1 - 2026-10-26

A fix release: the message that refuses text where a vector is expected reads correctly.

### Fixed

* `KivaDB does not compute embeddings: ...`, the answer to text sent where a vector is expected, contained runs of spaces left over from line wrapping. It now has single spaces, in the shell, the structured requests and `KIVA.SIMILAR`.

## 1.6.0 - 2026-10-26

### Added

* Documents: `json append`, `json insert` and `json incr` change an array or a number in place; `json type`, `json len` and `json keys` inspect a document. Paths accept negative positions (`.tags[-1]` is the last element) and quoted field names (`["read.time"]`) ([10.8, 10.9](DOCUMENTATION.md#108-arrays-and-numbers-in-place)).
* Hash indexes: `create index on <pattern> (<field>) using hash` answers `=` in constant time; the default stays the sorted (B-tree) index, which also answers `>` and `<`. `indexes` shows each index's kind and number of distinct values ([11.4](DOCUMENTATION.md#114-indexes--making-queries-fast)).
* `create index on user:*.address.city`: the field can follow the pattern after a dot, as in conditions.
* `memory pin "<pattern>"` keeps the values of matching keys in memory, now and after every write, whether the value cache is on or not; `memory unpin` lets them go. Pinned patterns are saved with the workspace ([18.5](DOCUMENTATION.md#185-pinning-keys-in-memory)).
* `memory` shows where a workspace's memory goes: keys, secondary and vector indexes, pinned values, value cache.
* `storage.max_memory_mb` bounds a workspace's memory: past it, the value cache gives its memory back, then new keys are refused with an `OOM:` error; changing, reading and deleting keys still work ([18.6](DOCUMENTATION.md#186-a-memory-bound)).
* Redis clients reach vector search and the semantic cache: `KIVA.SIMILAR <pattern>.<field> <vector> [LIMIT n] [METRIC m] [EF n] [EXACT]` answers `[key, score, document]` triples, and `KIVA.API <json>` runs any structured request (`similar`, `semantic`, `query`...) ([23.10](DOCUMENTATION.md#2310-vector-search-and-the-semantic-cache-from-a-redis-client)).
* Queries say plainly what the language leaves out: joins (two patterns, or a value taken from other documents), sorting pipelines (`order by`, `distinct`, `having`...), other operators (`in`, `like`...) and distributed queries are refused with what to do instead, in the shell and in the structured `query` request ([11.6](DOCUMENTATION.md#116-what-a-query-does-not-do)).
* Redis clients: unsupported commands are refused by an explicit list of the commands KivaDB runs, with the error Redis gives for an unknown command. For cluster, Redis functions, replication and module commands, the message also says why and what to use instead. `COMMAND COUNT` and `COMMAND LIST` answer that list ([23.3](DOCUMENTATION.md#233-supported-commands)).
* `resp.max_collection_members` (100 000 by default) bounds the members of a Redis set, sorted set or list: a command that would go past it changes nothing and answers an error; a key above a lowered limit can still shrink ([23.8](DOCUMENTATION.md#238-sets-and-sorted-sets)).
* Redis clients: the rest of the classic commands on strings, keys, hashes and lists. Strings: `GETRANGE` (`SUBSTR`), `SETRANGE`, `MSETNX`. Keys: `RENAMENX`, `COPY`, `TOUCH`. Hashes: `HINCRBYFLOAT`, `HSTRLEN`, `HSCAN`, `HRANDFIELD`. Lists: `LPUSHX`, `RPUSHX`, `LSET`, `LREM`, `LTRIM`, `LINSERT`, `LPOS`, `LMOVE`, `RPOPLPUSH`, `LMPOP`, for reliable queues and capped lists. Checked with ioredis and node-redis ([23.3](DOCUMENTATION.md#233-supported-commands)).
* Redis clients: the JSON commands of RedisJSON (`JSON.SET`, `JSON.GET`, `JSON.MGET`, `JSON.DEL`, `JSON.TYPE`, `JSON.NUMINCRBY`, `JSON.NUMMULTBY`, `JSON.ARRAPPEND`, `JSON.ARRINSERT`, `JSON.STRLEN`, `JSON.ARRLEN`, `JSON.OBJLEN`, `JSON.OBJKEYS`). A document written this way is an ordinary KivaDB document: `HGET`, `get where`, secondary and vector indexes and `KIVA.SIMILAR` all reach it. Checked with node-redis (`client.json`, RESP3) and ioredis ([23.11](DOCUMENTATION.md#2311-json-documents-redisjson-commands)).

* Vector search filters on other fields: `get similar doc:*.embedding to [...] where lang = "fr" and year >= 2020`. Conditions use the six comparisons and are joined by `and`. KivaDB chooses how to apply them. A secondary index that leaves few documents pre-filters them, and the search stays exact even with HNSW. An exact vector index ranks the vectors, then reads the documents closest first until enough pass. An HNSW search asks the graph for more until enough pass. The last line says which was used. The structured request takes `"filter"` (`{"lang": "fr"}` or `[{"field", "cmp", "value"}]`), and `KIVA.SIMILAR` takes `FILTER <field> <op> <value>` ([12.9](DOCUMENTATION.md#129-filtering-on-other-fields)).
* `vector.max_ram_mb` bounds the memory of a workspace's vector indexes. By default (`auto`) it is half the machine's RAM, or half the container's memory limit when that is lower; `0` removes it. A write that would add a vector past the bound is refused with an `OOM:` error, and so is an index too large, whose build stops before it fills the memory. `storage.max_memory_mb` now counts a new vector even for a key that already exists. `indexes` and `memory` show the memory of each vector index and the bound ([12.10](DOCUMENTATION.md#1210-memory-bounds)).
* Text sent where a vector is expected is refused with `KivaDB does not compute embeddings: ...` in the shell, the structured requests and `KIVA.SIMILAR`. `indexes` counts the documents whose field holds something else than numbers. The limits of the vector search are documented: in memory, on one machine, simple filters, embeddings computed by the application ([12.11](DOCUMENTATION.md#1211-what-the-vector-search-leaves-to-your-application)).
* `tune vector index on <pattern> (<field>) [recall 0.95] [limit 10] [sample 100] [queries "<pattern>"]` measures an HNSW index on its own data and keeps the smallest `ef_search` that finds the requested share of the true closest vectors. The queries are indexed vectors searched leave-one-out, or the application's real queries. On 100 000 vectors this gave 40 where real queries needed 25 (grouped data), and 1600 where they needed 1600 (evenly spread data). The value is saved with the index and used by every search that gives no `ef`. It can also be set by hand (`tune ... ef_search N | default`, `create vector index ... using hnsw ef_search N`), and `indexes` shows it. The format of the `.graphs` file is documented ([12.12](DOCUMENTATION.md#1212-tuning-an-hnsw-index)).
* `tune vector index ... auto` keeps an HNSW index tuned as its data grows or shrinks: the recall asked for is saved with the index, and the engine measures it again in the background each time the number of vectors doubles or halves since the last tuning. On an index still empty, it starts once there are enough vectors. `tune ... auto off` stops it, and so does an `ef_search` set by hand. `indexes` shows it ([12.12](DOCUMENTATION.md#1212-tuning-an-hnsw-index)).
* Semantic cache: a missed `semantic lookup` gives a `since` token, and `semantic store ... since <token>` does not keep an answer when a key it `depends` on changed after the lookup, while the model was computing it (`Not stored: kb:42 changed after the lookup...`). The structured request takes `"since"` and answers `"stored"` and `"stale"`. The Node.js driver gains `semanticCached(cache, question, compute)`, which runs lookup, model and store this way. `get similar` with `semantic.cache.enabled` no longer keeps results when the data changed during the search. `semantic stats` also shows the answers dropped by writes and the distinct dependencies ([12.8](DOCUMENTATION.md#128-semantic-cache-reuse-answers-to-the-same-question)).
* Queries combine comparisons with `and`, `or`, `not` and parentheses: `get where user:*.age > 26 and (role = "dev" or role = "admin")`. The pattern is named once. A missing field makes its comparison false. The engine intersects the keys of several indexes for `and`, and merges them for `or` when every part has an index; `explain` shows `INDEX MERGE` and suggests the missing indexes. A query with one comparison runs as before ([11.1](DOCUMENTATION.md#111-the-condition), [11.5](DOCUMENTATION.md#115-explain--see-how-a-query-will-run)).
* Aggregates: `avg`, `min` and `max` join `count` and `sum`, and all five take `group by <field>`. They can read another field than the condition's (`avg user:*.salary where role = "dev"`) or every document of a pattern (`count user:* group by city`). They fold the documents in one pass, without keeping them ([11.3](DOCUMENTATION.md#113-aggregates-count-sum-avg-min-max-group-by)).
* Structured requests: `query` and `explain` take `"where"`: `{"and": [...]}`, `{"or": [...]}`, `{"not": {...}}`, `{"field", "cmp", "value"}` or `{"role": "dev"}`. The new `aggregate` request computes several aggregates at once, named in `"compute"`, per `"group_by"` ([20.4](DOCUMENTATION.md#204-structured-requests-for-tools)).
* HNSW indexes can keep their vectors quantized: `create vector index ... using hnsw quantize sq8` keeps one byte per number instead of four. The graph computes its distances on those bytes. A search asks it for twice the results wanted and orders them with the full vectors of the documents, on disk, so the scores returned are exact. `exact` reads the documents' vectors. Measured on 100 000 vectors of 128 dimensions: 44 MB instead of 81, the same results, 0.85 ms per search instead of 0.79. The setting is saved in `.meta` and in the graphs file (older graphs files are still read), and `indexes` shows it ([12.13](DOCUMENTATION.md#1213-quantization-a-quarter-of-the-memory-for-the-vectors)).
* Vector search through an HNSW graph tests its `where` filter while walking the graph, instead of searching, filtering, and searching again for more. Nodes that fail are walked through but never fill the list of best candidates. Conditions a secondary index serves are tested against its keys without reading documents. On a filter one document in five passes, with an index on its field, a search takes 8.8 ms and reads 10 documents, instead of 48 ms and 1 397. Without an index it finds more of the true closest vectors (99.8 % instead of 96.6 % on a rare filter), but reads more documents ([12.9](DOCUMENTATION.md#129-filtering-on-other-fields)).
* Redis clients: blocking commands `BLPOP`, `BRPOP`, `BRPOPLPUSH`, `BLMOVE`, `BLMPOP`, `BZPOPMIN`, `BZPOPMAX`, `BZMPOP` wait for data up to their timeout without holding up the other clients. Writes from any source wake them: Redis clients, the shell, the REST API, scripts, replication. `WAIT` waits for replicas ([23.12](DOCUMENTATION.md#2312-blocking-commands-and-watch)).
* Redis clients: `WATCH` and `UNWATCH`. `EXEC` answers nil and applies nothing when a watched key was written, deleted, created, flushed or expired since `WATCH`, whatever changed it. The check is made under the same lock as the writes of `EXEC`. Checked with node-redis (`WatchError`) and ioredis ([23.12](DOCUMENTATION.md#2312-blocking-commands-and-watch)).
* Redis clients: streams. `XADD`, `XLEN`, `XRANGE`, `XREVRANGE`, `XDEL`, `XTRIM`, `XSETID` and `XREAD` (with `BLOCK`). Consumer groups: `XGROUP`, `XREADGROUP`, `XACK`, `XPENDING`, `XCLAIM`, `XAUTOCLAIM`. And `XINFO`. A stream is a KivaDB object, bounded by `resp.max_collection_members` ([23.13](DOCUMENTATION.md#2313-streams)).

### Fixed

* Expiry times are kept to the millisecond. `PX`, `PEXPIRE`, `PSETEX`, `PXAT` and `GETEX PX` were rounded up to the next second, so a key could live up to a second longer than asked; `PTTL` and `PEXPIRETIME` now answer exact milliseconds. Data files, hint files and replication streams written by earlier versions hold seconds and are converted when read ([23.3](DOCUMENTATION.md#233-supported-commands)).
* Semantic cache: a document that expired with its TTL did not drop the answers that `depends` on it; they are now dropped at the latest a second after its deadline. Every write compared its key with every cached answer (14.7 ms per write with 10 000 answers, so at most about 70 writes per second). The answers are now indexed by what they depend on, and the patterns by their text before the first wildcard. A write looks up only the patterns that can match it: 0.4 µs with 10 000 answers, and 0.6 µs when each of them has a pattern of its own.
* Redis clients: a refused command inside `MULTI` was queued and failed only at `EXEC`, while the rest of the transaction was applied. It is now refused at once, and `EXEC` answers `EXECABORT` and applies nothing, as in Redis.
* Two clients changing different fields of the same document at the same moment (`json set`, `json delete`) could lose one of the two changes. Each field change is now one step on the server.
* Replication covers every workspace, not only `default`. A replica receives each workspace's keys, the creation and removal of workspaces, and the users, stored scripts and triggers. Every workspace is read-only on a replica. All workspaces share one stream and one sequence, so `wait`, the lag and failover elections count them together. `replica status` lists the replicated workspaces. Triggers no longer run on a replica, whose master's trigger writes arrive with the rest. Indexes and the configuration stay per node. A master refuses replicas of older versions, which would apply every workspace to their default one: upgrade the replicas first ([25.9](DOCUMENTATION.md#259-what-is-replicated)).

### Changed

* `storage.sync_writes`: group commit. The fsync runs outside the workspace lock, and the writes that arrive during one fsync are confirmed together by the next one, so many clients writing at once share the disk confirmations instead of waiting for one each. Every write is still on disk before it is acknowledged. One writer at a time runs the fsync while the others wait for its end, and the ones it covered return all together. The fsync first lets the writes already on their way finish appending, for up to a millisecond. It goes through a file handle of its own, so on Windows the writes no longer wait for it to end. Redis clients wait for the disk after their command, without holding up the other clients. Measured on Windows with an SSD, writes per second: from 690 to 8 400 with 16 threads (12 writes per fsync instead of 3) and from 880 to 20 000 with 64 threads (52 per fsync) in the engine; from 216 to about 5 000 with 16 Redis clients. One client writes as before. The setting applies immediately, and `stats` shows the writes per disk sync ([17.2](DOCUMENTATION.md#172-what-survives-what)).
* `tune vector index` no longer holds the workspace while it measures: it takes the lock for one search at a time and lets waiting writes go first. During a tuning of 4 000 vectors, the slowest write waited 7.6 ms.
* A write that leaves an indexed field unchanged no longer updates that index.
* Data files now hold expiry times in milliseconds. This version reads files of earlier versions; an earlier version reading files of this one would take those expiry times for dates thousands of years away, and keys with a TTL would no longer expire there. Keep a backup before upgrading if you may need to go back.
* `and`, `or`, `not`, `group by`, `min` and `max` are no longer refused in queries. Joins and sorting pipelines (`order by`, `distinct`, `having`) still are, with the same explanations ([11.6](DOCUMENTATION.md#116-what-a-query-does-not-do)).
* Vector indexes keep their memory count up to date on every write, instead of walking every vector every few seconds.
* `explain get where ...` refuses words after the condition instead of ignoring them.
* Expired keys are removed every second (every 5 seconds before) and the sweep no longer walks every key: keys with a TTL are kept sorted by deadline, so it visits only the expired ones, by batches of 10 000. Before, as soon as one key had a TTL, each sweep read the whole index while holding the workspace lock.

## 1.5.1 - 2026-10-22

A fix release: `config set` now says correctly when a setting applies, and `help` lists every command.

### Fixed

* `config set` no longer says that a setting "takes effect the next time KivaDB starts" when it applies at once: `semantic.*`, `failover.*`, `tls.required`, `http.stream` and `server.max_clients` are read again at every use.
* The shell's `help` lists the `semantic` commands, `using hnsw` and `failover status`.

## 1.5.0 - 2026-10-22

Security, availability and AI: data encrypted on disk, a new master elected by itself when the old one dies, backups in one command, an approximate vector index and a semantic cache for large AI workloads, and Redis sets, sorted sets and publish/subscribe.

### Added

* `set <key> <value> if not exists`: creates the key only when it is absent; otherwise the command succeeds with `OK: <key> skipped (already exists)` instead of an error. Works with a type, `ttl` and `and` chains.
* `del <key> if exists`: deletes the key when present; otherwise prints `OK: <key> skipped (does not exist)`. Both clauses are case-insensitive and make `.kva` files safe to run again. DOCUMENTATION 6.1, 6.5 and 3.10.
* **Redis publish/subscribe**: `PUBLISH`, `SUBSCRIBE`, `UNSUBSCRIBE`, `PSUBSCRIBE`, `PUNSUBSCRIBE` (Redis glob patterns), `PUBSUB CHANNELS | NUMSUB | NUMPAT`, in RESP2 and RESP3 (push frames). An application with several instances can tell all of them that something changed. Channels belong to the user's database 0 workspace; messages published inside `MULTI` or a Lua script leave when it commits; a listener that stops reading is disconnected after 65,536 waiting messages. Checked with ioredis and node-redis. DOCUMENTATION 23.7.
* **HNSW vector index** for large collections: `create vector index on doc: (embedding) using hnsw [metric cosine|dot|l2] [m N] [ef N]`. `get similar` uses it automatically when the pattern and metric match, and compares only a small part of the vectors (on 100 000 vectors of 128 dimensions grouped by topic, 0.5 ms per search instead of 105 ms, with the same 10 results). New search options `ef N` (accuracy against speed) and `exact`; the JSON request takes `"ef"` and `"exact"` and answers `"approximate"`; the Node.js driver passes them on. The graph follows every write; deleted vectors are cleaned up by a rebuild in the maintenance task; it is saved in `<file>.graphs` and read back at start, replaying only the writes made since. DOCUMENTATION 12.5 and 12.6.
* **Encryption at rest** (AES-256-GCM, with `ring`): every record (key and value together, header authenticated), the index snapshot (`.hint`) and the HNSW graphs (`.graphs`), in every workspace and the system database. The key comes from `KIVADB_ENCRYPTION_KEY` or a file named by `encryption.key_file`, never from the data folder; `kivadb keygen` creates one. `compact` encrypts data written before the key; `KIVADB_ENCRYPTION_OLD_KEY` / `encryption.old_key_file` allow a key rotation; `encryption.required` refuses to start without a key and refuses records in clear. The `encryption` command shows the key id and what is still in clear. With encryption on, the local shell keeps no history file. DOCUMENTATION 16.12.
* **Redis sets and sorted sets**: `SADD`, `SREM`, `SMEMBERS`, `SISMEMBER`, `SMISMEMBER`, `SCARD`, `SPOP`, `SRANDMEMBER`, `SMOVE`, `SINTER` / `SUNION` / `SDIFF` (and `STORE`), `SINTERCARD`, `SSCAN`; `ZADD` (`NX`, `XX`, `GT`, `LT`, `CH`, `INCR`), `ZINCRBY`, `ZREM`, `ZSCORE`, `ZMSCORE`, `ZCARD`, `ZCOUNT`, `ZLEXCOUNT`, `ZRANK` / `ZREVRANK` (`WITHSCORE`), `ZRANGE` (`BYSCORE`, `BYLEX`, `REV`, `LIMIT`, `WITHSCORES`), `ZRANGESTORE`, `ZREVRANGE`, `ZRANGEBYSCORE`, `ZRANGEBYLEX` and their reverse forms, `ZPOPMIN` / `ZPOPMAX`, `ZREMRANGEBYRANK` / `SCORE` / `LEX`, `ZRANDMEMBER`, `ZSCAN`, `ZUNION` / `ZINTER` / `ZDIFF` (and `STORE`, `WEIGHTS`, `AGGREGATE`), `ZINTERCARD`. Stored as marked KivaDB objects (saved, replicated, encrypted), `TYPE` answers `set` / `zset`, RESP3 doubles and sets. Checked with ioredis and node-redis. DOCUMENTATION 23.8.
* **`kivadb backup` and `kivadb restore`**: `kivadb backup --output <file.tar.gz> [-w <workspace>]` saves the whole home (configuration, users, every workspace, audit log, TLS certificate) or one workspace, with a SHA-256 per file in a manifest and `<file>.sha256` for the archive. It works while KivaDB runs (the server makes the backup; each data file is copied as it was at one instant, writes go on, compactions wait) or stopped. `kivadb restore <file.tar.gz> [--target <dir>] [--as <name>] [--check]` checks the archive, every file and every record before writing anything, then restores a home into an empty folder (with `kiva.conf` pointing at its new place) or a workspace under its name or another; it refuses while KivaDB runs on that home. Shell: `backup to "<file>" [workspace <name>]` (admin). DOCUMENTATION 17.7.
* **Automatic failover** (`failover.auto`): the master sends heartbeats listing the group's nodes; a replica that hears nothing for `failover.down_after_ms` checks the master itself, asks the other replicas, and a new master is elected only when a majority of the group (the master counted; `failover.quorum` to change it) sees the master down. The most up-to-date replica wins (then `failover.priority`, 0 = never); votes are given once per epoch to a candidate at least as up to date. The new master tells the other replicas, which resynchronise from it; an old master that comes back finds the newer epoch and becomes a replica. `failover status`, `READONLY` errors that name the master, `master_host` / `master_port` in Redis `INFO`. `replica promote` and `failover to` open a new epoch too. DOCUMENTATION 25.8.
* **Semantic cache**: `semantic lookup | store | stats | clear` keep answers (of a language model, say) next to the embedding of their question, and give them back for a question at cosine similarity >= `semantic.cache.threshold` (0.98), or for the same text word for word. Named caches per workspace, HNSW search, TTL (`semantic.cache.ttl`, per entry), invalidation as soon as a key matching a `depends` pattern is written or deleted, `semantic.cache.max_entries`. With `semantic.cache.enabled`, `get similar` reuses the results of a near-identical recent query. Structured request `semantic`, `cached` in `similar` answers, `semanticLookup` / `semanticStore` in the Node.js driver. In memory, not saved. DOCUMENTATION 12.8.

### Changed

* A write refused on a replica names the master: `READONLY: this node is a replica of <host:port>; ...`.
* `config set required ...` (without its section) is ambiguous since `encryption.required` exists next to `tls.required`: write `config set tls.required true` or `config set encryption.required true`, as the documentation does.
* The distance functions of vector search are vectorised: exact searches are faster too.

### Fixed

* Reopening a database right after it was closed could fail with "database is locked" on Linux, while another process was being started (it inherits open files for an instant). KivaDB now waits up to half a second for such a lock.

## 1.4.0 - 2026-10-20

Encrypted connections, commands over several lines with `.kva` command files, and a one-command installer for Windows.

### Added

* **Encrypted connections (TLS)** on every port, next to plain ones: 2911 (shells, Kiva Studio, drivers), 8080 (HTTPS) and 6379 (`rediss://`). KivaDB recognises a TLS connection by its first byte.
* A certificate created on first start in `<home>/tls/`, or your own (`tls.cert`, `tls.key`). `kivadb tls` shows it and its SHA-256 fingerprint; `kivadb tls renew` creates a new one; `tls.hosts` adds names or addresses to it.
* `tls.required`: refuse plain connections from other machines (the machine itself may still connect in clear). Refused clients get a message in their own protocol.
* Shell: `kivadb --connect host:port --tls`. A new server's fingerprint is shown and, at a terminal, the user is asked to trust it; trusted servers are kept in `known_servers`. `--tls-fingerprint` for scripts. A remote connection without `--tls` prints a warning.
* Rust client: `Client::connect_tls`. Node.js driver: the `tls` option, with `fingerprint`. Replicas: `replication.master_tls` and `replication.master_fingerprint`.
* DOCUMENTATION 16.11.
* **Commands over several lines** in the shell: while a `{` or `[` is open, Enter goes on to a ` ...>` line instead of failing with "Unclosed bracket"; the command runs when the document closes. The history keeps it whole.
* **`.kva` command files**: commands as in the shell, documents over several lines, `#` and `--` comments. `kivadb run file.kva` runs them (also on a remote server with `--connect`), reports a failing command with its file and line and stops there (`--keep-going` to go on); `source "file.kva"` does the same in the shell. DOCUMENTATION 3.9 and 3.10.
* **`install.ps1`**: installs the latest release on Windows in one PowerShell command, `irm https://raw.githubusercontent.com/fomadev/kiva-db/main/install.ps1 | iex`: SHA-256 check, `%LOCALAPPDATA%\Programs\KivaDB` without administrator rights, the folder added to the user's `PATH`. `KIVADB_INSTALL_DIR`, `KIVADB_VERSION`, `KIVADB_NO_MODIFY_PATH`.

### Changed

* Commands piped into the shell (`kivadb < file`) end with exit code 1 when one of them failed (it was always 0).

## 1.3.0 - 2026-10-18

Redis compatibility, made complete enough to replace Redis where it serves as short-lived memory (sign-up codes, locks, rate limits, sessions). Checked with the current ioredis, node-redis, rate-limiter-flexible and rate-limit-redis, now part of the CI (`tests/redis-clients`).

### Added

* Lua scripts through Redis: `EVAL`, `EVALSHA`, `EVAL_RO`, `EVALSHA_RO`, `SCRIPT LOAD | EXISTS | FLUSH`, with `redis.call`, `redis.pcall`, `redis.error_reply`, `redis.status_reply`, `redis.sha1hex`. Scripts run in the Lua sandbox, all or nothing.
* RESP3: `HELLO 3` is accepted (node-redis 5 and later use it), with maps and nulls.
* Numbered databases: `SELECT 1` to `15` (or `/n` in a URL) use the workspaces `db1` to `db15`; database 0 is the user's own workspace.
* `EXPIRE`, `PEXPIRE`, `EXPIREAT`, `PEXPIREAT` accept `NX`, `XX`, `GT` and `LT`.
* `GETEX`, `EXPIRETIME`, `PEXPIRETIME`, `TIME`.
* DOCUMENTATION 23.5 (Lua scripts) and 23.6 (replacing Redis in an application).

### Fixed

* Commands from several Redis connections could interleave: two clients running `INCR` on the same key at the same moment could lose a step, which weakens a rate limiter. Redis commands now run one at a time, as in Redis.
* `EXPIRE ... NX` (and the other options) were ignored, so a rate-limit window restarted at every attempt.

## 1.2.0 - 2026-10-18

### Changed

* `compact` is no longer reserved to administrators in every case: an `editor` may now compact their own personal workspace (`u_<name>`). Everywhere else, including workspaces a user can write to through a grant, it still needs the admin role, because it rewrites a whole shared file. Viewers still cannot run it. The refusal message says where it is allowed.

## 1.1.0 - 2026-10-18

### Added

* `kivadb update` installs the latest release in place of the running executable, on Windows, Linux and macOS. The download is checked against the release's `SHA256SUMS.txt` and the new executable is run once before it replaces the old one; on any problem nothing is changed. `kivadb update --check` only tells whether a new version exists.
* A discreet check for new releases: in a terminal, KivaDB looks at GitHub at most once a day, in the background, and mentions a new version when the command ends. Scripts, services, `kivadb serve` and containers never trigger it. `KIVADB_NO_UPDATE_CHECK=1` turns it off.
* `install.sh`: installs the latest release on Linux or macOS in one command, `curl -sSL https://raw.githubusercontent.com/fomadev/kiva-db/main/install.sh | bash`.
* `KIVADB_RELEASES_URL` points `kivadb update` to a mirror of the releases.

## 1.0.1 - 2026-10-16

### Improved

* `kivadb start` now shows every listener it started, not only the TCP port:

  ```text
  KivaDB started (pid 13052).
    TCP port : 2911
    REST API : http://127.0.0.1:8080
    Redis    : port 6379
  ```

  When the REST API or the Redis port cannot start (another program already uses the port), the reason is shown on those lines instead, and KivaDB runs without it.

### Internal

* The test suites read the version from `Cargo.toml` instead of repeating it.

## 1.0.0 - 2026-10-15

First release: typed key-value and document database in one executable, with secondary indexes and queries, vector search, transactions, workspaces, users and roles, compaction, compression, replication with failover, Lua scripts and triggers, change notifications, REST API, Redis protocol, Prometheus metrics, Docker image and the Node.js driver. See [DOCUMENTATION.md](DOCUMENTATION.md).
