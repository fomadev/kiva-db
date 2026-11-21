# KivaDB 2.0.0-rc.5: projects and collections, and the work around them

This is an audit, and a design for the fifth release candidate. It covers:

- the model project → collection, and how data is laid out on disk;
- the defects of `get where` found during the audit;
- expiry events in the change journal, finer grants, import and export;
- metrics, the shell, the drivers;
- full-text and hybrid search, as a study only.

**Nothing here is implemented.** Each lot waits for the maintainer's approval of the decisions in section 12. `v2.0.0-rc.4` (tag `d173b56`, published 2026-11-20) is the reference for comparisons and compatibility.

Statements about the code refer to `main` at `d173b56`.

## 1. What exists today

### 1.1 Workspaces and projects

| Concept | In the code | Where |
| :--- | :--- | :--- |
| Workspace | One engine (`Engine`), one data file `store.kiva` with its `.meta`, `.hint`, journal and graphs, in one folder | `Instance::engine`, `instance.rs` |
| Its name | `analytics`, or `<project>.<workspace>` (`travel.places`): letters, digits, `_`, `-`, 64 at most per part, one `.` | `auth::valid_workspace_name` |
| Its folder | **Written in `kiva.conf`**, section `[workspaces]`, `travel.places = <path>`. `default` is `storage.default_workspace` (`<home>/data`). A new workspace gets `<home>/workspaces/<name>` unless a path is given | `Config::workspace_dir`, `Instance::create_workspace` |
| Project | A record `project:<name>` in the system database (replicated), with its date of creation. It holds no data of its own; its workspaces are those whose name starts with `<name>.` | `Instance::create_project`, `projects`, `remove_project` (refused while not empty) |
| Rename and move | `workspace move a to travel.a`: the name changes in `kiva.conf`, the folder stays. The grants follow. It is crash-safe: a record is written first, and `recover_moves` finishes or takes back a move cut in the middle | `Instance::rename_workspace` |
| Session | One workspace at a time, or none (2.0: an account without grants lands in no workspace). `workspace switch`, alias `workspace use` | `session.rs`, `exec_admin.rs` |
| Backups | The archive names workspaces, not folders: `workspaces/<name>/store.kiva`. A restore puts them in `<home>/workspaces/<name>` | `backup.rs` |
| Replication | Workspaces are announced by name (`workspace_changed`); a replica creates them in its own home | `repl.rs` |

**The finding that matters most.** The link between a name and a folder is already a setting, not a rule: the engine opens whatever folder `kiva.conf` names. A hierarchical layout can therefore be given to new workspaces without moving a single existing file. Existing ones can be moved later, by choice, with the crash-safe mechanism `workspace move` already has.

### 1.2 Grants

* **Roles:** `admin`, `editor`, `viewer`.
* **Access levels:** `read` and `write`, given per **scope**: `*`, `<project>.*`, `*.<workspace>`, `<project>.<workspace>` or `<workspace>` (`auth::normalize_scope`, `scope_covers`). A grant on `travel.*` covers the workspaces created in `travel` later.
* **Maintenance:** an editor who can write every workspace of a project manages that project (`User::manages`).
* **Absent:** grants on a prefix of keys, denials, custom roles.

What was asked as `grant read on baak.* to "analyst"` **already works**.

### 1.3 Queries and indexes

* `get where <pattern>.<field> <op> <value>`, combined with `and` / `or` / `not`. Since rc.4, also `order by random() limit N` on a pattern alone.
* **Secondary indexes:**
  * kinds: B-tree, hash, vector (HNSW);
  * unique, on up to 8 fields (`MAX_UNIQUE_FIELDS`), `ignore case`;
  * a unique index on several fields answers an `and` of equalities (`composite_plan`).
* **No non-unique index on several fields.**
* No full-text index.

### 1.4 Change journal and TTL

* **Journaled:** writes, deletions, `del all keys`, inside transactions, per workspace, with named consumers and cursors (`changes.rs`).
* **Not journaled:** **expirations** (manual, 22.x). A key whose TTL ran out disappears without a change.
* **What exists for expiry:** the engine already calls listeners when it sees one (`Engine::expired`, `add_expiry_listener`), today only to invalidate the semantic cache.

### 1.5 Import, export, metrics, shell, drivers

| Area | Today |
| :--- | :--- |
| Import / export | **None.** Only the physical backup (`kivadb backup` / `restore`) |
| Prometheus | `kiva_requests_total` by 4 types (get, set, del, other), one histogram `kiva_request_duration_seconds` for all requests, plus errors, storage, memory, compactions, clients, replication lag, change journal and consumer lag. No metric on the semantic cache, HNSW, index use or expiries |
| Shell | 10 usage texts start with `Usage:` and are given to `Cur::finish`, which writes `Usage:` before them: `Unexpected 'x'.\nUsage: Usage: ...` (`exec.rs` 1861, 2196, 2331, 2564; `exec_admin.rs` 739, 841, 868, 929, 1043, 1109) |
| Python driver | `connect(workspace=...)`, switched again after a reconnection |
| Node.js driver | **No workspace option at all** (`index.js` has none) |
| Kiva Studio | A separate repository (Flutter), with 160 mentions of "workspace": any renaming reaches it |

### 1.6 Two defects of `get where` found by this audit

Tried on rc.4:

```text
kiva> set user {"city": "Kinshasa"} and user:1 {"city": "Kinshasa"} and userx:1 {"city": "Kinshasa"}
kiva> get where user.city = "Kinshasa"
user: {"city":"Kinshasa"}
(1 matches)
kiva> explain get where user.city = "Kinshasa"
  Strategy  : PREFIX SCAN
  Estimated : 3 of 3 keys examined
  Tip       : create index on user (city)
```

1. **The answer is right, the reading is not.** A pattern without a wildcard names one key, and only `user` is returned. But the plan reads every key that starts with `user` (`user:1`, `userx:1`...) before keeping one: a full scan of the prefix, for one key. It should read that key and nothing else.
2. **The tip is wrong:** an index on one key helps nothing.

Neither expression should be rewritten into the other: `user` is a key, and `user:*` a family of keys. What is missing is an exact reading, and a hint when an exact key matches nothing while `user:*` would (section 4.1).

## 2. The proposed model

```
Instance
└── Project   baak            (a name; holds collections; grants on baak.*)
    ├── Collection   baak.cache
    ├── Collection   baak.places
    └── Collection   baak.users
```

* **A collection is what a workspace is today:** one engine, isolated, holding keys, documents, vectors, its indexes, schemas and journal. There is **one concept**, not two. `collection` becomes its name in the commands, the manual and the APIs; `workspace` stays accepted (section 3).
* **A project** is what it is today, a record that groups collections and carries grants. It may be empty. Creating one makes **no** collection in it.
* **The qualified name** `baak.places` identifies a collection everywhere: grants, backups, replication, `kiva.conf`. Two projects may each have a collection `places`.
* **Collections outside any project** (`lab`, `default`, and those of 1.x homes) stay valid. Forcing them into a project would change names that applications use. Section 12, D3.

## 3. Commands and APIs

| Proposed | Today | Note |
| :--- | :--- | :--- |
| `project create baak` | same | unchanged |
| `project create baak collections (cache, places, users)` | — | new; atomic or taken back after a crash (5.2) |
| `collection create baak.places [path]` | `workspace create baak.places` | the old form stays |
| `collection list [baak]`, `project list` | `workspace list [baak]` | the old form stays |
| `use baak.places` | `workspace switch baak.places` | `use` is new; the old form stays |
| `use baak` | — | selects the project, **no collection**: a data command then says to choose one (`use baak.places`). `collection list` lists the project's |
| `collection rename baak.cache to baak.sessions`, `collection move baak.sessions to dzoket.sessions` | `workspace move` | the same crash-safe mechanism, two names for clarity |
| `collection remove baak.sessions` | `workspace remove` | unregisters, files kept, as today |
| `project remove baak` | same | refused while it holds collections, as today |
| The prompt `kiva[baak.places]>` | `kiva>` | shows the project or the collection in use |

**APIs:**

- Structured requests and REST gain `"op": "collections"` and the field `"collection"` in `whoami` and in answers. `"workspace"` keeps being answered, with the same value, until 3.0.
- REST takes `?collection=` as well as `?workspace=`.
- Redis keeps `SELECT n` and `SELECT baak.places`.
- Node.js gains `connect({ collection })`, Python `connect(collection=...)`, with `workspace` kept as an alias.

## 4. Queries and indexes

### 4.1 Exact keys in `get where` (P0, a fix)

* A pattern without `*` or `?` is read as one key: a plan `KEY LOOKUP`, 1 key examined, no index tip.
* When that key is missing and `<pattern>:*` has keys, the answer adds one line: `(0 matches; 'user' is one key: for the keys user:..., write user:*)`.
* The results do not change, only the reading and the hint.
* Tests: with and without indexes, an exact key, a prefix, two collections.

### 4.2 Non-unique indexes on several fields (P2)

* **Today:** a unique index on several fields serves equalities.
* **Proposed:** `create index on order: (city, created_at)`, a B-tree on the tuple. It would serve an equality on the first fields, then a range or an order on the next, page by page, with its counts in `explain analyze`.
* **Before code:** a cost model and a benchmark. Proposed for 2.1, not rc.5 (D8).

### 4.3 Full-text and hybrid search (J, study)

* **Needed:**
  * an inverted index, Unicode normalisation, tokenisation by language, scoring (BM25), phrases;
  * text filtered by conditions;
  * text combined with vectors (HNSW), with a rule to merge scores (reciprocal rank fusion is the usual safe start), and a stable order for pages.
* **Proposed grammar**, next to `get where` and `get similar`, to be specified: `get where product:* match description "laptop" and price < 1000 [similar embedding [...]] limit 20`.
* **This is a lot of work.** Proposed: a separate specification now, code in 2.1 (D9).

## 5. Physical layout and migration

### 5.1 New collections

* **New collections go in `<home>/projects/<project>/<collection>/`.** A collection outside a project goes in `<home>/collections/<name>/`. `default` stays in `<home>/data`.
* The folder is still written in `kiva.conf`. The engine is not changed: it opens the folder it is given.
* **Folder names are checked, not trusted:**
  * they are made only from names that `valid_name_part` accepts: letters, digits, `_`, `-`, so no `.`, `/`, `\` or `..`;
  * a folder that already exists is refused rather than reused;
  * names that differ only by capitals are refused within one project, since Windows and macOS do not tell them apart.

### 5.2 Creating several at once

* `project create baak collections (cache, places, users)` writes a record `pending:project:baak` in the system database first, then the project and each collection, then removes the record.
* A start after a crash finds the record and finishes the creation. Nothing is lost by finishing, since all is new and empty.

### 5.3 Existing collections

**Nothing moves on upgrade.** A collection of rc.4 (`<home>/workspaces/baak.places`) keeps its folder, its name and its grants.

* An **explicit** command, `kivadb relocate [--dry-run]`, KivaDB stopped, can later move them into the new layout:
  * it lists what it would move;
  * then, one collection at a time:
    1. it copies the folder;
    2. it checks every file by its checksum;
    3. it writes the new path in `kiva.conf`;
    4. it removes the old folder.
  * A record in the system database lets a start after a crash finish or take back the move in progress, as `recover_moves` does.
* It is optional, and can come after rc.5 (D5).

### 5.4 Going back

* The names in `kiva.conf` do not change, only paths for new collections. So **rc.4 opens a home of rc.5** as long as no data of a new kind is written; rc.5 adds none.
* An rc.4 does not know `pending:` records; it ignores them.
* Projects created by `project create ... collections` are ordinary projects.
* **To be proved, not assumed:** a test with the real rc.4 (`KIVADB_RC4_BIN`), as rc.3 has.

## 6. Expiry events in the change journal (P1)

* **When:**
  * an event `expire` is written **when the engine sees the expiry**: on a read, a scan, or the periodic sweep (`purge_expired`);
  * it is not written at the theoretical millisecond.
  * It carries `expires_at` (the moment the TTL ran out) and the time it was seen.
* **Once:**
  * the engine sees an expiry once, then drops the key; one change per key and expiry in one journal;
  * a replica writes its own when it sees the expiry. Replicas do not replicate expiries today; to be checked.
* **After a crash:** an expiry seen but not yet journaled is seen again at the next start, and journaled then. The guarantee is "at least once" to consumers, as for the other changes.
* **Transactions:** an expiry is outside any transaction.
* **Semantic cache:** it already invalidates on expiry; nothing changes there.
* **Format:** a new operation code in the journal. An rc.4 reading that journal would not know it: to be tested, and it may need the journal's own version (D6).

## 7. Grants on a prefix of keys (P1, the riskiest)

* **Proposed:**
  * `grant read on baak.places keys "place:public:*" to "guest"`: a grant limited to keys of one prefix, in one collection;
  * no denials in rc.5: grants only add, which keeps the rule simple (the highest grant wins), as today.
* **Every path must check it:**
  * keys, ranges, scans and queries (rows outside the prefix are not returned, and not counted in aggregates);
  * vector search, `changes`, Lua (`kiva.get`, `kiva.scan`), Redis (`KEYS`, `SCAN`, `MGET`, `EVAL`), REST and structured requests;
  * semantic cache dependencies.
* **Out of reach for such a user:** maintenance commands (`compact`, `create index`, `schema`), backups and `changes enable`; they stay for full grants.
* **Risk:** one forgotten path leaks data. Proposed: a test that sends every command of the cheat sheet through an account with a prefix grant (D7).

## 8. Import and export (P1)

* **Commands:**
  * `kivadb export <collection> [--keys <pattern>] -o file.ndjson`;
  * `kivadb import <collection> file.ndjson [--dry-run] [--replace]`.
* **Format:** one line per key, `{"key": ..., "type": ..., "value": ..., "expires_at": ...}`. The value is in extended JSON (C5), so decimals, dates and typed documents keep their types.
* **Optional:** `--with-schema` adds the schemas and index definitions, in a first line.
* **Import:**
  * it writes in batches;
  * `--dry-run` checks everything and writes nothing;
  * a second run is safe: the same file gives the same keys.
* **Not a backup:** export is logical, readable by other tools, filtered; the backup stays the physical copy. The manual will say which to use when.

## 9. Metrics (P1)

* **New metrics:**
  * `kiva_requests_total` and `kiva_request_duration_seconds` by kind of command, a fixed list of about 15 (get, set, query, aggregate, similar, transaction...), never by key or user;
  * `kiva_semantic_cache_{hits,misses,evictions}_total`;
  * `kiva_vector_index_bytes`, `kiva_index_scans_total{kind}`, `kiva_expired_keys_total`;
  * `kiva_backup_failures_total`, `kiva_transaction_conflicts_total`.
* **Quantiles** stay with Prometheus (`histogram_quantile`), from histograms.
* `stats` and `doctor` show the same counts.

## 10. Shell and drivers

* **Fix now (P0):** `Usage: Usage:` in the 10 texts of 1.5. The rule becomes one place adding `Usage:`.
* **Proposed:**
  * the prompt with the collection in use;
  * one form for errors of syntax, type, bounds, grants and unknown names: `Error: <what>. Usage: <form>`;
  * completion of commands and names in the interactive shell only; scripts and `-e` stay as they are.
* **Drivers:** a table of parity (shell, structured, REST, Node.js, Python), checked by a test that lists the operations of `api.rs`.
  * The Node.js driver gains `collection` and the operations it lacks.
  * The Python driver gains `collection`; it already has vectors, transactions and changes, which are not done again.

## 11. Lots, risks and versions

| Lot | Content | Risk | Proposed for |
| :--- | :--- | :--- | :--- |
| A | This note, and the decisions of section 12 | — | now |
| P0 | `Usage: Usage:`, exact keys in `get where` (4.1) | low | rc.5, first |
| B | Collections: commands, `use`, prompt, APIs, aliases of `workspace`, `project create ... collections` | medium (wording everywhere, Studio) | rc.5 |
| C | The new layout for new collections (5.1); `relocate` later (5.3) | medium; low without `relocate` | rc.5 (layout); `relocate`: D5 |
| E | Expiry events in `changes` | medium (journal format) | rc.5 |
| G | Import and export NDJSON | low | rc.5 |
| H | Metrics, shell errors, prompt | low | rc.5 |
| I | Drivers: `collection`, parity table | low | rc.5 |
| F | Grants on a prefix of keys | **high** (every path) | D7: rc.5 or 2.1 |
| D | Non-unique indexes on several fields | medium | 2.1 |
| J | Full-text and hybrid search | high, large | specification now, code 2.1 |

**Version:**

- The maintainer chose rc.5.
- Nothing in lots P0 to I breaks a home or an application: old commands, fields and folders stay.
- So rc.5 does not need 3.0. Removing `workspace` from the APIs would, and is not proposed.
- Then 2.0.0 stable after rc.5, without rc.6 if nothing blocks.

**Tests for rc.5:**

- the homes of 1.7.1, rc.2, rc.3 and **rc.4** (a new fixture, written by the published rc.4) open;
- the real rc.4 reads a home written by rc.5;
- crash tests for `project create ... collections` and for `relocate`;
- backups and restores into the new layout;
- replication of projects and collections;
- benchmarks against rc.4.

## 12. Decisions for the maintainer

| # | Question | Recommendation | Alternative |
| :--- | :--- | :--- | :--- |
| D1 | The name of the concept | **`collection`**, one concept; `workspace` accepted everywhere as an alias until 3.0 | Keep `workspace` (no renaming, less work, Studio unchanged) |
| D2 | `use baak` on a project | Selects the project, no collection; data commands ask for one | Enter a collection of the same name or the only one (implicit, rejected by the brief) |
| D3 | Collections outside a project (`lab`, `default`, 1.x) | Stay valid, as they are | Move them into a project `default` (renames, breaks applications) |
| D4 | Layout of new collections | `<home>/projects/<p>/<c>`, `<home>/collections/<c>` outside a project; paths still in `kiva.conf` | Keep `<home>/workspaces/<p>.<c>` |
| D5 | Existing folders | Do not move on upgrade; `kivadb relocate --dry-run` explicit, after rc.5 | `relocate` in rc.5 |
| D6 | Expiry events | `expire` when seen, with `expires_at`, at least once | Leave expiries out of the journal (today) |
| D7 | Grants on a prefix of keys | **2.1**, with the test of every command; rc.5 keeps grants per project and collection | In rc.5 (more risk before stable) |
| D8 | Non-unique indexes on several fields | 2.1, after a cost model | rc.5 |
| D9 | Full-text and hybrid | Specification in rc.5, code in 2.1 | Code in rc.5 |
| D10 | Import and export | NDJSON with extended JSON, `--dry-run`, in rc.5 | JSON only |
| D11 | Order of the lots | P0 → B → C → E → G → H → I, one PR each, then rc.5 | — |
