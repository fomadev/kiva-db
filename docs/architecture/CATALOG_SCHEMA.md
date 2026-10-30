# Catalogs and schemas: what KivaDB 2.0 already has

An audit, written before any code, of a proposal to organise KivaDB as **server → catalog → schema → data**, with grants on `*.*`, `<catalog>.*` and `<catalog>.<schema>`, and connections that land straight in one schema. Nothing here is implemented: it says what exists in `2.0.0-rc.2`, what is missing, what it would cost, and what to decide first.

Every statement below was checked in the code, by a test, or by running `2.0.0-rc.2`; the last section lists what was not.

## 1. The answer in short

The hierarchy already exists, under other names: a **project** is the proposed catalog, a **workspace** the proposed schema. KivaDB 2.0 has the namespaces, the grants with wildcards, the connections into one workspace, the move between projects and the isolation, with tests. What the proposal adds is mostly **vocabulary** (`CATALOG`, `SCHEMA`, `USE`, `SHOW`) and a few real gaps (section 5).

No change to the storage engine or the file format is needed for any of it.

The proposal also asks whether each schema should share one storage engine or be a physically separate database. That is already decided, and the second: **each workspace is a database of its own**, with its own folder, data files, indexes, journal and backups (section 3).

## 2. The proposal, item by item, against 2.0

| Proposed | In 2.0 | Where | Tested by |
| :--- | :--- | :--- | :--- |
| `CREATE CATALOG "travel"` | `project create travel` | `Instance::create_project` (`src/instance.rs`); stored as `project:<name>` in the system database, so replicas know it | `projects_group_workspaces_and_grants_name_them` (`tests/commands.rs`) |
| `CREATE SCHEMA "travel"."places"` | `workspace create travel.places` | `Instance::create_workspace`; refused if the project does not exist | same |
| Same schema name in two catalogs | `travel.places` and `school.places` are two workspaces | the name is the pair `<project>.<workspace>` (`auth::valid_workspace_name`) | same |
| `SHOW CATALOGS`, `SHOW SCHEMAS FROM "travel"` | `project list`, `workspace list travel` | both show a user only what they can reach | same |
| `SHOW CURRENT CONTEXT` | `whoami`, `workspace current` | `whoami` also answers `access`, `can_write`, `can_compact` | `tests/commands.rs`, `tests/network.rs` |
| `USE "travel"."places"` | `workspace switch travel.places` | `Session::switch_workspace` (`src/session.rs`): reads the stored account at each switch, refused inside a transaction | `users_roles_and_workspaces` |
| `DROP CATALOG` (refused when not empty) | `project remove travel`, refused while it holds a workspace | `Instance::remove_project` | `projects_group_workspaces_and_grants_name_them` |
| `DROP SCHEMA` | `workspace remove travel.places`: unregisters it and leaves its files on disk | `Instance::remove_workspace` | `workspaces_and_configuration` |
| `ALTER SCHEMA ... MOVE TO CATALOG` | `workspace move travel.places to school.places` | `Instance::rename_workspace`: the files stay where they are, the grants on that workspace and the default workspaces follow, refused while a session is in it, replicas copy it again | `projects_group_workspaces_and_grants_name_them` |
| `GRANT ... ON "travel"."places"` | `grant read\|write on travel.places to "app"` | `auth::normalize_scope`, `auth::scope_covers` | same |
| `GRANT ... ON "travel".*`, also for schemas created later | `grant write on travel.*` | the scope is kept as written and checked at each switch | same ("covers its workspaces created later") |
| `GRANT ... ON *.*` | `grant write on *.*` (also `*`, `all`) | stored as `*` | same |
| `*.places` | `grant read on *.places` | covers the workspace `places` of every project, including future projects | same |
| Several grants per user | yes, any number of scopes | `User::grants`, a map from scope to read or write | same |
| `REVOKE` | `revoke write on <scope>` (to read), `revoke read on <scope>` (removed) | exact scope only: revoking `travel.places` does not cut into `travel.*` | same |
| Roles and elementary permissions | roles `admin`, `editor`, `viewer`; grants `read`, `write`; a user gets the highest grant that covers the workspace, bounded by the role | `User::access` (`src/auth.rs`) | `users_roles_and_workspaces` |
| Deny by default | a new account reaches no workspace | 2.0 change, [CHANGELOG](../../CHANGELOG.md) | `users_roles_and_workspaces` |
| Login into one schema | `<user>@<project>.<workspace>` in the shell (`auth`, `-u`), over HTTP, the binary protocol and Redis (`redis://app%40travel.places:secret@host:6379`); refused if the user cannot reach it | `src/session.rs`, `src/resp.rs` | `network_logins`, `projects_group_workspaces_and_grants_name_them`, `tests/redis-clients/check.mjs` |
| Redis `SELECT` | `SELECT travel.places` by name; `SELECT n` is the workspace `db<n>` | `src/resp.rs` | `redis_numbered_databases_are_workspaces` |
| REST | `?workspace=travel.places` on each request | `src/http.rs` | `tests/network.rs` |
| Python driver | `connect(..., workspace="travel.places")`, entered again after a reconnection | `drivers/python/src/kivadb/client.py` | `drivers/python/tests/test_driver.py` |
| Node.js driver | no `workspace` option; `connect({ user: "app@travel.places" })` works (run against `2.0.0-rc.2`: lands in `travel.places`; `app@school.places` without a grant is refused) | `drivers/node/index.js` | not tested, not documented in its README |
| Kiva Studio | workspaces grouped by project in the switcher, grants on scopes, default workspace, sessions in no workspace | fomadev/kiva-studio#4 | its `test/kivadb2_test.dart` |
| Context per connection, never global | each session has its own workspace and access level | `Session::ws`, `Session::ws_priv` | `projects_group_workspaces_and_grants_name_them` (sessions of several users in different workspaces) |
| Change of context inside a transaction | refused: `cannot switch workspace inside a transaction` | `Session::switch_workspace` | not tested |
| Revoked access applied | at the next switch or login; a long read of the change journal (`wait`) checks the stored account again at each turn (`check_reads_now`) | `src/session.rs`, `src/api.rs` | `tests/changes.rs` |

## 3. Storage: each workspace is a database of its own

* A workspace is registered in `kiva.conf` (`[workspaces]`, name → folder) and opened as its own `Engine`. Next to its data file are a `.meta` file (the definitions of its indexes, vector indexes and document schemas, and the settings of its journal), a `.graphs` file (HNSW graphs) and its change journal (`changes::dir_of`). `default` lives in `data/`, the others in `workspaces/<name>/` unless created with a path.
* Keys, unique indexes, vector indexes, TTL, transactions, document schemas, queries and the change journal belong to one engine. So `travel.places` and `school.places` can hold the same key, and a unique index in one does not see the other. No code compares two engines.
* The **system database** holds what is shared: users and their grants, projects, stored scripts and triggers.
* Backups are per home or per workspace (`kivadb backup`, `kivadb restore ... --as`); replication copies every workspace.

The proposal's other option, one storage engine shared by the schemas of a catalog, would be a step back: a shared file mixes the data and the compaction of applications that are kept apart today, and a project could no longer be moved, backed up or removed one workspace at a time. **Keep one database per workspace.**

## 4. Terminology: the decision to take first

Calling projects *catalogs* and workspaces *schemas* changes no behaviour. It weighs as follows.

**For.** People who know PostgreSQL or SQL Server read `catalog.schema` at once.

**Against.**

* **`schema` already means something in KivaDB 2.0**: `schema set user: {...}` defines the rules of documents ([11.8](../../DOCUMENTATION.md#118-schemas-rules-for-the-documents-of-a-pattern)), released in `2.0.0-rc.1` and `rc.2`. `CREATE SCHEMA "travel"."places"` next to `schema set user:` gives one word two meanings in the same shell. Renaming the document schemas instead (`DocumentSchema`, `rules`) would break a command just published.
* A SQL schema holds tables of one database. A KivaDB workspace *is* a whole database, with its own files. The SQL word suggests the shared engine of section 3, which KivaDB does not have.
* `project` and `workspace` are in the commands, the REST parameter (`?workspace=`), the structured requests (`workspaces`, `default_workspace` in `users`), the Python driver (`workspace=`), Kiva Studio, the configuration (`[workspaces]`, `storage.default_workspace`), the 33 chapters of the manual, and every application written for 2.0. Two vocabularies would have to live together for years.
* The rename comes after two release candidates whose upgrade guide ([Chapter 33](../../DOCUMENTATION.md#chapter-33--upgrading-from-kivadb-1x-to-20)) teaches `project` and `workspace`.

**Options.**

| | What | Cost |
| :--- | :--- | :--- |
| **A (recommended)** | Keep `project` and `workspace`. Close the gaps of section 5. | None for users |
| B | Add `catalog` as another name for `project` (`catalog create`, `catalog list`), keep `workspace`; never `schema` for a workspace | Two names for one thing in the manual and the answers |
| C | Rename everything to `catalog` / `schema` in a 3.0, with the document schemas renamed too and the old forms kept for a deprecation period | A major version, every interface and driver, a migration guide |

## 5. What is really missing

Independent of the names.

| # | Gap | Verified by | Proposal |
| :--- | :--- | :--- | :--- |
| 1 | **No project rename.** `workspace move` moves one workspace; a project is renamed only by creating the new one, moving each workspace, then removing the old one | no `project rename` / `project move` in `src/exec_admin.rs` | `project rename travel to trips`: moves each workspace with the rules of `workspace move` (refused while one is in use), then the `project:` record, and rewrites the grants on `travel.*` |
| 2 | **No delegated administration of a project.** An editor with *write* on `travel.*` may create workspaces in it and run `compact`; creating indexes, schemas or users stays with admins | [16.4](../../DOCUMENTATION.md#164-workspaces-and-grants), `Need::Owner` | Decide whether an *owner* level on a project (`grant owner on travel.*`) is wanted, and what it covers: indexes and document schemas of its workspaces, its workspaces' removal, not users or the server |
| 3 | **Names leak to authenticated users.** A logged-in user without access tells an existing workspace from a missing one: `no access to workspace 'school.places'` against `workspace 'secret.nothing' does not exist` (run on `2.0.0-rc.2`). Before login nothing leaks: a wrong password always answers `Login failed.` | `Session::switch_workspace` checks existence before access | For a non-admin, one answer for both: `no workspace 'x' that you can reach` |
| 4 | **`workspace list` shows the folder of each workspace** to non-admins: the server's paths | run on `2.0.0-rc.2` | Show folders to admins only |
| 5 | **`kiva.conf` is not written atomically.** It holds the list of workspaces, and is written with `fs::write`: a crash or a full disk while it is written can leave it truncated, and the workspaces it named are then unknown at the next start (their files stay) | `Config::save` (`src/config.rs`) | Write a temporary file in the same folder, sync it, rename it over `kiva.conf`. A fix of its own, before any of the rest |
| 5b | **The `.meta` file of a workspace is not written atomically either, and a damaged one is ignored in silence.** `Engine::save_meta` writes it with `fs::write`; at the next start, if it is not valid JSON, the workspace opens with no secondary index, no unique index, no vector index definition and no document schema, and says nothing: unique values and schema rules are then no longer enforced | `Engine::save_meta`, and the reading in `Engine::open` (`if let Ok(meta) = ...`) (`src/engine.rs`). Reproduced on `2.0.0-rc.2`: a unique index on `user:* (email)`, `store.kiva.meta` cut to 20 bytes, then at the next start `indexes` answers `No secondary indexes in this workspace.` and a second `a@x` is saved | Same atomic write; and a `.meta` that cannot be read stops the opening with an error naming the file, rather than dropping the constraints. Not tied to catalogs: a fix for 2.0.0 |
| 6 | **`workspace move` is two writes.** It renames the workspace in `kiva.conf`, then rewrites each account whose grants or default name it. A crash in between leaves grants on the old name: the data is safe, but those users lose access until an admin grants it again | `Instance::rename_workspace` | Rewrite the accounts first (a grant on a missing workspace is harmless), then `kiva.conf`; or log the move and finish it at start |
| 7 | **The Node.js driver has no `workspace` option** and does not document `user@workspace` | `drivers/node/index.js`, its README | `connect({ workspace: "travel.places" })`, entered again after a reconnection like the Python driver; document `user@workspace` |
| 8 | **No connection URL for the KivaDB protocol.** Neither driver parses a URL; Redis clients use `redis://user%40project.workspace:pw@host:6379` | both drivers | `connect("kivadb://app:pw@host:2911/travel.places")` in both drivers, the path naming the workspace. Not for `redis://`, where the path is a database number |
| 9 | **No explicit deny** (`deny write on travel.secrets`) | `User::grants` holds read or write only | Keep it that way: with deny by default and exact revokes, a deny rule adds an order of precedence to learn and test, for little |

## 6. Compatibility and migration

* **Option A** changes no stored data and no interface. Gaps 1 to 8 add commands and options; gaps 3 and 4 change answers that programs may read (an error message, the column of `workspace list`), which goes in the changelog.
* **Option B** adds aliases only.
* **Option C** needs a migration guide like Chapter 33, and the old forms for a deprecation period. The files need not change: `kiva.conf` may keep `[workspaces]`, and the system database `project:`, behind new words.
* In every option, the homes of 1.7.1 and 2.0 open as they are; no step is destructive or automatic.

## 7. Plan

The order follows the risks, not the vocabulary.

1. **Gaps 5 and 5b** (`kiva.conf` and `.meta` written atomically, a damaged `.meta` refused) and **gap 6** (`workspace move` in a safe order): fixes, tested by a crash in the middle of the write.
2. **Gaps 3 and 4**: what a user without access learns.
3. **Gap 7**, then **gap 8** if wanted: the drivers.
4. **Gap 1** (`project rename`), and **gap 2** once its scope is decided.
5. The vocabulary, if option B or C is chosen.

Each is a pull request of its own, with its tests. Gaps 5, 5b and 6 are fixes and should go into `2.0.0`; the others can wait for 2.1.

## 8. Not verified

* The Python driver and Kiva Studio against a project rename or an owner level: neither exists yet.
* How common Redis clients other than ioredis, node-redis and `redis-cli` treat a user name holding `@`.
* The cost of the access check (`User::access`, one lookup per grant at each switch) with many grants: it is not measured; it runs at login and at each switch, not at each command.
