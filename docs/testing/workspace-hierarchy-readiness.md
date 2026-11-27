# Workspace hierarchy readiness

**NOT READY**

Startup validates staged hierarchy metadata and exposes typed identity lookup.
The preflight checks registered physical paths, while data access still uses
legacy registrations.

| Area | Status | Evidence or remaining work |
| --- | --- | --- |
| Architecture before and target | PASS | [Architecture audit and migration plan](../architecture/WORKSPACE_HIERARCHY.md) |
| Model validation | PASS | Catalog tests cover typed IDs, mandatory parents, case-insensitive uniqueness, distinct homonyms and malformed snapshots |
| Migration mapping | PASS | Tests cover standalone stores, an occupied `legacy` name and refusal of orphaned or conflicting input |
| Startup validation and lookup | PASS | Integration tests reopen staged IDs, preserve routing and reject a malformed record |
| Catalog storage | PASS | Tests cover encrypted and unencrypted reopen, rollback on validation errors and rejection of a stale writer |
| Crash recovery | NON TESTÉ | The store reuses the engine transaction format. Crash tests spanning catalog and collection files are outstanding |
| Migration preflight | PASS | Missing or shared store paths are rejected and valid registered directories are retained |
| Hierarchical activation | NON TESTÉ | The runtime continues using legacy routes and grants |
| Engine, CLI and network integration | NON TESTÉ | Session context, CRUD, navigation, compatibility protocol, Rust API integration and drivers are outstanding |
| Security and concurrency | NON TESTÉ | The model validates parent relationships. Cross-workspace authorization, revocation and concurrent administrative operations are outstanding |
| Backup, replication, and doctor | NON TESTÉ | Remain bound to the existing storage names |
| Historical fixture preview | PASS | Mapping and reopening of staged metadata checked on copies of 1.7.1, encrypted 1.7.1, and release candidates 2, 3 and 4. Two absent personal stores in the unencrypted 1.7.1 fixture are rejected before being materialized on the test copy |
| Benchmarks and migrated fixtures | NON TESTÉ | No end-to-end migration or scale comparison has been run |
| Multiplatform CI | NON TESTÉ | Verify this branch's CI before evaluating platform coverage |

## Changes and verification

Updated `src/catalog.rs`, `src/engine.rs`, `src/instance.rs`, `tests/compat.rs`,
`tests/workspace_hierarchy.rs`, the architecture document and this report.
The encrypted transaction writer now authenticates the final commit header. This does not repair previously damaged records. No files or commands were removed and the record format is unchanged. Startup now returns an error if a
present staged catalog cannot be decoded or uses an unsupported version.

Run `cargo test --locked --test workspace_hierarchy --test compat --test project_context`
to verify the startup and preview behavior. The preview does not verify the
contents of every data file, snapshot concurrent mutations, or replace a
verified backup.

The five targeted suites passed 49 tests. Formatting and Clippy across all targets passed locally.

The offline staging procedure described below leaves routing disabled. Before this feature can be
enabled, verify the migration and rollback on isolated copies of supported
homes and run the concurrency, crash recovery and cross-interface tests
specified in the architecture work.

## Phase 2 offline migration

**NOT READY for hierarchical serving.** This phase adds an offline command
and an independent versioned staging manifest. Apply and rollback preserve
legacy storage and authorization. On Linux, a checksum-verified RC10
release binary generated a home and reopened it after apply and rollback.

| Verification | Status | Scope |
| --- | --- | --- |
| Offline staging and rollback | PASS | Apply, verify and rollback preserve historical bytes across 1.7.1, encrypted 1.7.1, RC2, RC3 and RC4 fixture copies |
| Backup integrity | PASS | Tampering and backup creation errors are refused without restoring or overwriting live files |
| Concurrency and path identity | PASS | Home and store locks, missing files and shared physical store identity |
| Interruption recovery | PASS | Before and after atomic publication and withdrawal under crash injection |
| Encrypted transaction regression | PASS | The new test fails on `fb73764` and passes with the corrected engine. Also verifies reopen and process crash after append |
| Hierarchical grant translation | NON TESTÉ | Existing grants stay active. No hierarchical grant adapter is activated |
| Historical executable interoperability | PASS | RC10 on Linux read the original order after apply, then read both the order and a later write after rollback. Other historical executables were not run for this scenario |
| Power-loss durability on Windows | NON TESTÉ | Atomic visibility and process crash recovery do not prove hardware durability |

The staging manifest is separate from the legacy engine files and does not
participate in backup or replication for live hierarchical serving. The next
phase must integrate session and authorization boundaries before changing
data routing.

## Phase 3 session and administration preview

**NOT READY for hierarchical serving.** The experimental shell mode stores
per-connection IDs for workspace, project, and collection and exposes
metadata-only CRUD. A staged migration is read-only. Its legacy registrations
are frozen through the application while data can still be served via legacy
routes.

| Verification | Status | Scope |
| --- | --- | --- |
| Preview navigation | PASS | Strict names, qualified paths, parent reset, missing parents and homonyms |
| Administrative metadata | PASS | Atomic creation batches, explicit cascade, default protection, stable identities, restart |
| Session isolation | PASS | Separate native TCP contexts, restricted login, and binary and structured data access refused |
| Changes in another session | PASS | Rename follows identity, drop falls back to a surviving parent, and recreated names do not rebind |
| Staged migration boundary | PASS | Preview reads the staged catalog without editing it, and legacy topology commands are refused |
| Hierarchical data grants | NON TESTÉ | Authorization and data engine identity routing remain legacy |
| Live hierarchical storage and driver integration | NON TESTÉ | Requires grant, protocol and maintenance integration before activation |

## Phase 4A physical layout preview and export

**NOT READY for hierarchical serving.** A staged backup can be exported
into the managed collection paths under a separate new directory. Empty
workspace and project directories contain no user store. The hierarchy
preview shows planned workspace paths and exposes resource-specific help.

| Verification | Status | Scope |
| --- | --- | --- |
| FS03–FS05, FS12–FS13 | PASS for offline export | Collection snapshot with empty parent directories, byte equality, and legacy rollback preserving the export |
| FS14 | PASS for offline export | Interrupted copy stays unpublished and retries into a new destination |
| FS15–FS16 | PASS on Linux for traversal and symlink refusal | Windows reparse-point check awaits a dedicated junction test |
| CLI01–CLI04 | PASS in preview | Scoped lists, explicit missing-project error, and resource help |
| Active layout lifecycle | NON TESTÉ | Online allocation, engine reopen, rename and retired storage remain outstanding |
| AUTH01–AUTH04 and DATA01–DATA04 | NON TESTÉ | Authorization and routing still use the legacy implementation |
| Performance and capacity preflight | NON TESTÉ | No phase-specific benchmarks or free-space estimator |

The export does not turn into a runnable database. Protocols and maintenance
must be integrated before any cutover to the proposed filesystem layout.

## Phase 4B managed storage lifecycle

**NOT READY for hierarchical serving.** The local administrator can
explicitly enter managed storage mode in an unstaged, standard test home.
The managed catalog has a durable pending snapshot. Recovery uses directory
and store identities to finish physical changes before publishing it.

| Verification | Scope |
| --- | --- |
| Physical lifecycle | Collection initialization, workspace/project/collection rename, non-destructive retirement, recreation with a fresh identity, and restart |
| Paths and ownership | Case collisions, traversal rejection, symlink rejection on Unix, and a Windows-only real junction test |
| Concurrency | Conflicting creations and refusal to move an open store |
| Recovery | Interrupted directory and store creation, moves, retirement, and catalog publication |
| Engine compatibility | Encrypted store creation and reopen through the existing engine, and refusal to adopt a substituted store |
| Remaining work | Live authorization, routing, staged migration cutover, restore/reattachment, managed backup, replication and client integration |

UNC and extended Windows paths, cross-volume relocation and sudden
power loss have no dedicated coverage. The managed catalog is an
administration record. The final cutover to a serving catalog remains a
subsequent integration.

## Phase 5 authorization and grants

**NOT READY for hierarchical serving.** The authorization model governs
managed metadata. Staged historical grants are projected onto exact
collection identities in the read-only view. In Phase 5, hierarchy modes could only manage metadata. The architecture document records the
before/after migration matrix.

| Area | Verification |
| --- | --- |
| Least privilege | Direct and inherited grants, viewer ceilings, separate metadata administration, and denied policy delegation |
| Session isolation | Qualified name denials match unknown resources. Lists, current context and structured identity hide inaccessible metadata |
| Lifecycle | Revocation across established sessions, concurrent metadata operations, rename, retirement and account or resource recreation |
| Recovery | Policy persists after restart and crash tests distinguish precommit and committed revocation |
| Corruption | Unsupported or malformed grant records prevent startup |
| Remaining | Permanent authorization cutover, data-engine enforcement, backup and all protocol adapters |

## Phase 6 local data routing

**NOT READY for production.** `hierarchy data` enables local collection data
commands in standard, unstaged homes with listeners disabled.

| Area | Verification |
| --- | --- |
| DATA01–DATA09, DATA11 | Collection reads and writes, missing selection, homonymous resources, restart, documents and native dates, index and schema isolation, TTL, vector search, transactions, change journal and integrity checks |
| DATA12 | Compaction is refused pending crash-safe publication of the replacement store identity |
| AUTH01–AUTH06, AUTH08 | Read, Write and Manage decisions, qualified selection, revoke during a session and before commit, and fresh resource identities |
| AUTH07 | Account identity and recreation tests in `hierarchy_grants` |
| LIFE01–LIFE04 | Rename, retirement, stale selection and empty recreation. Active transactions prevent concurrent topology changes |
| DATA10, CRASH01–CRASH02 | Encrypted managed writes with implicit and explicit commits. Recovery covers incomplete and complete batch records |
| CRASH03 | Managed administration recovery suite covers layout and policy changes |
| CONC01–CONC03 | Concurrent local data calls and grant changes, with completion checks |

The test suite simulates process crashes. Managed backup, restoration,
replication, doctor, protocol integration, drivers, staged cutover and
dedicated power-loss validation remain open.

### Release routing measurements

The Release library was exercised through `Session::execute` on Linux in a
shared overlay filesystem, with 5 visible Intel Xeon Platinum 8573C CPUs.
Both routes used synchronous writes. The benchmark used an authenticated
editor, with one legacy instance-wide Write grant or one managed Write grant
per collection. Each catalog had one project and 1, 32 or 128 collections.
Only one collection contained the workload, with six keys, one indexed
document and a 2D vector. File caches were warm, and instance restarts
reopened the stores. The startup measurement includes `Instance::open`.
Selection and first GET are timed together to include lazy engine opening
on either route. Later selections rotate through collection names.
Concurrent samples use two reader sessions and two writer sessions on the
same collection with disjoint write keys.

Values are median / p95 in µs. There were 15 starts and first accesses per
configuration, 100 repetitions per serial measurement and 400 per concurrent
measurement. The p95 is the nearest-rank percentile.

| Operation | Legacy 1 | Managed 1 | Legacy 32 | Managed 32 | Legacy 128 | Managed 128 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Instance startup | 1,269.6 / 6,470.6 | 1,039.2 / 6,288.8 | 1,046.2 / 19,918.5 | 2,996.1 / 29,416.6 | 1,230.1 / 26,736.6 | 16,450.5 / 39,375.2 |
| Select + first GET | 261.6 / 4,438.0 | 194.5 / 342.7 | 212.2 / 300.7 | 366.7 / 4,604.1 | 309.5 / 691.2 | 712.5 / 4,945.2 |
| Select | 8.2 / 9.0 | 8.1 / 9.2 | 4.9 / 5,684.5 | 13.1 / 20.7 | 2,146.3 / 12,501.0 | 22.0 / 72.7 |
| GET | 3.0 / 3.8 | 9.2 / 10.8 | 2.0 / 2.1 | 12.0 / 12.6 | 3.4 / 3.5 | 18.0 / 32.4 |
| Indexed query | 7.6 / 8.4 | 13.5 / 44.4 | 4.6 / 5.9 | 16.3 / 17.6 | 7.5 / 8.6 | 23.5 / 51.8 |
| Vector search | 8.3 / 9.9 | 14.7 / 24.9 | 4.8 / 5.1 | 16.6 / 18.8 | 8.0 / 10.9 | 23.0 / 26.3 |
| Update | 3,987.9 / 26,720.9 | 2,971.1 / 12,170.4 | 3,118.0 / 7,974.2 | 502.9 / 5,276.2 | 621.0 / 5,980.1 | 3,765.4 / 9,308.0 |
| Begin + update + commit | 3,849.5 / 15,085.2 | 2,174.3 / 9,636.6 | 3,532.3 / 12,495.6 | 3,715.3 / 13,102.6 | 3,620.8 / 7,361.4 | 3,596.8 / 6,455.1 |
| Report selected permissions | — | 7.5 / 8.4 | — | 10.1 / 10.8 | — | 16.3 / 33.4 |
| Concurrent read/write mix | 428.5 / 10,937.6 | 321.7 / 4,906.7 | 148.7 / 3,991.3 | 399.8 / 6,422.2 | 389.4 / 5,854.8 | 428.9 / 8,312.2 |

At 32 collections, process RSS was 8,264 KiB on the legacy route and 8,304
KiB on the managed route in a single process. The permissions row includes
command handling and current account, catalog and versioned policy checks.
The legacy route has no equivalent permission-report command. The high
variation in synchronous write times makes this run unsuitable for claiming
a throughput improvement. Managed operations serialize on the layout lock.
These measurements cover a small local workload and exclude network,
large data sets and backup/restore.


## Phase 6B managed engine hardening

**NOT READY for production.** Managed routing remains local and requires
explicit activation with the listener disabled.

- The admission test holds an authorized command open and verifies that
  commands in the same and a different workspace can complete while
  layout writers are excluded. Revocation, retirement and transaction tests
  verify visibility and write publication through existing sessions.
- The registry test sets the capacity to two engines and exercises saturation,
  transaction pins, LRU eviction, and reopening an evicted store.
- The compaction test interrupts ten prepare, copy, sync, swap, publication
  and cleanup boundaries with and without encryption. It verifies actual
  stored values, tombstones, TTL-visible values, index plans, vector results,
  schema metadata, change journal history and generation recovery.
- The platform failure model is process interruption. Power-cut tests,
  protocol adapters, driver integration, managed backup, restore, replication,
  doctor and migration cutover remain open.

### Phase 6B Release comparison

The [benchmark example](../../examples/hierarchy_routing_bench.rs) uses
`Session::execute` in Release with managed writes synced in both versions.
The baseline is commit `90f0c8c`, the merge of PR #168. The comparison used
Linux, an overlay filesystem and five visible Intel Xeon Platinum 8573C
CPUs in a shared runner. No process or disk cache was flushed between samples.

Each case has four workspaces, two projects per workspace, four editors
and 5, 36 or 132 stored grants. Up to four collections contain six keys
and a document with a secondary index and a two-dimensional vector. Startup
samples include open and clean shutdown. The open-or-hit sample rotates
through up to 128 selections and executes a GET, so includes both the cold
open path and the registry path. The concurrent workload starts two readers
and two writers with a barrier. In the distributed case each worker uses a
separate workspace when at least four collections are present. The 1-store
distributed case is another run of the concentrated workload. The compaction
case includes a first rewrite followed by repeated rewrites of the same
small populated store.

Run using `cargo run --locked --release --example hierarchy_routing_bench --
<new-scratch-directory> 1 32 128 1000 10000`. Build the same example with the
baseline library and append `--baseline` to omit commands unavailable in that
revision. Provision separate homes for each executable.

Latency is p50 / p95 / p99 in microseconds, using nearest-rank percentiles.
Sample counts per run are 12 for open and shutdown, 128 for selection and up
to 128 for select plus GET, 256 for warm reads and the four-thread workload,
64 for serial writes and transactions, 16 for policy changes and 8 for
compaction. Small samples give coarse tail estimates.

| Collections | Operation | Phase 6 | Phase 6B |
| ---: | --- | ---: | ---: |
| 1 | Open + shutdown | 5,196.62 / 17,800.32 / 17,800.32 | 4,709.10 / 49,848.44 / 49,848.44 |
| 1 | Qualified selection | 8.41 / 9.62 / 44.01 | 7.91 / 8.63 / 22.30 |
| 1 | Select + first access or hit | 873.55 / 873.55 / 873.55 | 296.10 / 296.10 / 296.10 |
| 1 | GET | 9.25 / 9.74 / 40.17 | 8.80 / 10.42 / 35.37 |
| 1 | SET | 537.71 / 1,250.82 / 2,613.09 | 450.75 / 5,094.59 / 27,401.34 |
| 1 | Indexed query | 13.79 / 15.00 / 51.12 | 12.96 / 15.36 / 140.41 |
| 1 | Vector search | 9.41 / 10.31 / 47.10 | 13.64 / 15.16 / 175.38 |
| 1 | Begin + update + commit | 506.27 / 4,722.01 / 13,027.62 | 456.62 / 1,623.22 / 7,039.42 |
| 1 | Permissions command | 5.07 / 7.87 / 28.31 | 7.15 / 8.54 / 77.83 |
| 1 | Mixed operations in one collection | 356.30 / 4,319.62 / 22,985.92 | 205.99 / 4,004.83 / 7,816.74 |
| 1 | Mixed operations across workspaces | 448.78 / 3,727.09 / 42,081.40 | 361.03 / 4,960.95 / 7,610.03 |
| 1 | Grant, revoke and GET | 1,442.20 / 4,027.31 / 4,027.31 | 855.89 / 7,706.37 / 7,706.37 |
| 1 | Compaction | — | 6,915.42 / 9,798.40 / 9,798.40 |
| 32 | Open + shutdown | 3,025.87 / 7,905.78 / 7,905.78 | 9,553.21 / 29,541.05 / 29,541.05 |
| 32 | Qualified selection | 7.69 / 12.55 / 91.90 | 5.27 / 5.99 / 28.19 |
| 32 | Select + first access or hit | 69.75 / 156.29 / 9,571.03 | 67.89 / 2,302.62 / 17,914.55 |
| 32 | GET | 8.21 / 8.40 / 11.71 | 8.86 / 9.58 / 31.98 |
| 32 | SET | 435.11 / 1,776.38 / 4,165.81 | 1,016.26 / 6,470.72 / 8,464.63 |
| 32 | Indexed query | 10.96 / 11.76 / 63.65 | 8.82 / 14.68 / 46.91 |
| 32 | Vector search | 11.23 / 20.54 / 127.90 | 8.48 / 8.76 / 27.60 |
| 32 | Begin + update + commit | 393.39 / 2,662.69 / 6,593.08 | 438.92 / 2,080.95 / 3,406.20 |
| 32 | Permissions command | 6.83 / 7.98 / 43.32 | 6.94 / 7.59 / 9.10 |
| 32 | Mixed operations in one collection | 334.06 / 4,299.62 / 23,506.72 | 294.48 / 6,800.01 / 15,565.16 |
| 32 | Mixed operations across workspaces | 278.86 / 1,389.75 / 7,125.38 | 72.94 / 12,978.85 / 44,175.90 |
| 32 | Grant, revoke and GET | 3,531.34 / 10,764.63 / 10,764.63 | 5,645.72 / 28,172.26 / 28,172.26 |
| 32 | Compaction | — | 5,701.78 / 30,919.15 / 30,919.15 |
| 128 | Open + shutdown | 8,253.16 / 10,070.10 / 10,070.10 | 15,210.14 / 35,933.64 / 35,933.64 |
| 128 | Qualified selection | 18.98 / 21.28 / 89.73 | 8.08 / 8.99 / 32.69 |
| 128 | Select + first access or hit | 695.27 / 2,891.10 / 6,128.09 | 1,042.48 / 4,973.91 / 11,722.90 |
| 128 | GET | 20.23 / 24.72 / 165.90 | 5.78 / 8.71 / 33.93 |
| 128 | SET | 391.35 / 605.20 / 817.00 | 970.83 / 8,146.78 / 14,479.43 |
| 128 | Indexed query | 15.82 / 34.11 / 89.26 | 8.44 / 14.58 / 125.87 |
| 128 | Vector search | 16.04 / 23.79 / 49.12 | 8.48 / 9.36 / 71.89 |
| 128 | Begin + update + commit | 541.52 / 1,375.85 / 3,100.65 | 2,251.18 / 15,569.67 / 34,493.32 |
| 128 | Permissions command | 17.09 / 18.65 / 71.40 | 4.98 / 8.88 / 81.27 |
| 128 | Mixed operations in one collection | 314.51 / 2,508.08 / 23,659.51 | 155.54 / 11,356.45 / 71,389.91 |
| 128 | Mixed operations across workspaces | 266.20 / 5,411.90 / 41,609.10 | 345.42 / 8,549.80 / 30,635.87 |
| 128 | Grant, revoke and GET | 9,141.27 / 18,088.77 / 18,088.77 | 14,455.19 / 237,070.58 / 237,070.58 |
| 128 | Compaction | — | 11,388.89 / 20,615.69 / 20,615.69 |
| 1,000 | Open + shutdown | 53,524.40 / 93,715.37 / 93,715.37 | 93,307.20 / 190,512.44 / 190,512.44 |
| 1,000 | Qualified selection | 83.91 / 123.85 / 483.74 | 5.65 / 6.76 / 20.97 |
| 1,000 | Select + first access or hit | 1,225.94 / 13,335.77 / 53,700.10 | 657.34 / 3,951.11 / 20,179.85 |
| 1,000 | GET | 61.09 / 83.17 / 124.15 | 5.84 / 6.16 / 15.94 |
| 1,000 | SET | 1,108.31 / 9,233.39 / 52,278.32 | 2,158.26 / 7,531.50 / 26,054.59 |
| 1,000 | Indexed query | 64.19 / 120.96 / 3,695.00 | 8.57 / 9.40 / 33.83 |
| 1,000 | Vector search | 64.15 / 91.44 / 249.07 | 8.66 / 21.25 / 103.25 |
| 1,000 | Begin + update + commit | 1,271.43 / 21,418.51 / 36,020.81 | 401.49 / 625.51 / 17,817.06 |
| 1,000 | Permissions command | 59.64 / 109.77 / 328.83 | 4.87 / 5.13 / 22.10 |
| 1,000 | Mixed operations in one collection | 390.07 / 15,695.51 / 173,330.63 | 314.29 / 853.95 / 3,838.71 |
| 1,000 | Mixed operations across workspaces | 359.66 / 7,941.44 / 19,157.71 | 336.90 / 928.74 / 2,456.54 |
| 1,000 | Grant, revoke and GET | 14,106.87 / 36,321.27 / 36,321.27 | 5,325.18 / 24,090.91 / 24,090.91 |
| 1,000 | Compaction | — | 4,381.21 / 5,559.82 / 5,559.82 |
| 10,000 | Open + shutdown | 1,608,679.58 / 2,825,246.32 / 2,825,246.32 | 933,698.95 / 2,111,178.62 / 2,111,178.62 |
| 10,000 | Qualified selection | 604.23 / 8,594.73 / 16,645.39 | 7.62 / 21.65 / 56.62 |
| 10,000 | Select + first access or hit | 6,906.40 / 52,611.01 / 79,632.93 | 796.59 / 2,541.20 / 7,117.17 |
| 10,000 | GET | 888.61 / 5,060.21 / 9,158.56 | 5.96 / 6.28 / 15.43 |
| 10,000 | SET | 7,603.56 / 28,780.09 / 35,997.85 | 464.07 / 974.21 / 2,084.03 |
| 10,000 | Indexed query | 968.72 / 9,151.14 / 32,921.16 | 8.55 / 8.79 / 28.32 |
| 10,000 | Vector search | 938.15 / 12,662.58 / 33,361.82 | 8.61 / 9.01 / 79.74 |
| 10,000 | Begin + update + commit | 10,731.77 / 35,767.91 / 85,810.55 | 418.45 / 728.21 / 845.03 |
| 10,000 | Permissions command | 903.04 / 9,591.29 / 32,944.68 | 4.96 / 5.09 / 7.01 |
| 10,000 | Mixed operations in one collection | 2,064.30 / 34,166.19 / 414,760.51 | 203.88 / 1,374.69 / 2,950.19 |
| 10,000 | Mixed operations across workspaces | 2,606.36 / 30,899.66 / 658,394.62 | 473.74 / 1,616.96 / 3,938.79 |
| 10,000 | Grant, revoke and GET | 14,786.73 / 40,711.65 / 40,711.65 | 8,903.25 / 21,870.86 / 21,870.86 |
| 10,000 | Compaction | — | 5,871.24 / 8,038.43 / 8,038.43 |

| Collections | Concentrated ops/s before → after | Distributed ops/s before → after | RSS KiB before → after | Descriptors before → after |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 1,655 → 2,512 | 1,723 → 1,434 | 7,712 → 7,812 | 15 → 14 |
| 32 | 2,238 → 1,251 | 2,545 → 473 | 8,084 → 8,092 | 78 → 77 |
| 128 | 2,796 → 667 | 1,571 → 912 | 8,652 → 8,736 | 78 → 77 |
| 1,000 | 565 → 5,358 | 901 → 6,625 | 10,976 → 10,748 | 78 → 77 |
| 10,000 | 163 → 5,277 | 198 → 3,696 | 31,832 → 31,028 | 78 → 77 |

RSS and descriptors are process snapshots after the workload. RSS may include
allocator retention from preceding cases. The registry held 32 open engines
at the larger sizes and evicted 100 idle engines in the 128-open sweep.
Disk-bound tails vary considerably on the shared runner. The measured GET
improvement follows elimination of per-request catalog cloning and grant
scanning. The startup path still verifies the physical store and marker for
each collection and checks for interrupted compaction. Open and shutdown were slower at 1,000 collections and faster at 10,000
in this run. The mixed results and shared filesystem make a general
startup improvement unverified. The hardening comparison preceded the
final write-through and failed-replacement cleanup changes. The separate
component measurements include the revised recovery checks.

### Authorization and catalog component costs

These Release samples use the 1,000-collection benchmark home with 132
grants. The ignored `managed_routing_component_costs` test reads that home
when `KIVADB_HIERARCHY_BENCH_HOME` is set. It measures each component in
isolation. The layout barrier sample is uncontended. The policy load
includes reading, deserializing, validating and indexing the grant record.
The catalog load includes filesystem validation. Comparisons under real
contention are represented by the mixed workload above.

| Component | Samples | p50 / p95 / p99 (ns) |
| --- | ---: | ---: |
| validated catalog load | 16 | 104,606,527 / 1,084,789,682 / 1,084,789,682 |
| policy load and index | 128 | 436,071 / 4,445,995 / 11,670,190 |
| catalog Arc lookup | 2,048 | 68 / 171 / 204 |
| collection ID and parent resolution | 2,048 | 53 / 59 / 64 |
| current account validation | 2,048 | 4,322 / 5,926 / 70,945 |
| versioned policy and account lookup | 2,048 | 157 / 176 / 224 |
| inherited permission | 2,048 | 153 / 185 / 212 |
| uncontended layout guard | 2,048 | 53 / 61 / 74 |


## Phase 7A native TCP

**NOT READY for production.** Native managed TCP requires explicit startup
and per-connection capability negotiation. The legacy native listener
remains available for historical homes.

| Coverage | Regression checks |
| --- | --- |
| NET01–NET08, NET12–NET15 | Login, negotiation, separate contexts, initial empty selection, homonymous collections, binary and JSON requests, restart and legacy compatibility |
| NET09–NET11, API02–API04 | Revocation, failed commit after account recreation, stale collection handles, rollback, disconnect and a crash just before the commit reply |
| NET16–NET22 | Malformed frames, slow reader of request bytes, TLS pinning, engine limit, disconnect cleanup, concurrent data, compaction and rename |
| NET23 | Push subscriptions are refused. Managed journal reads reauthorize each request |
| API01 | Rust workspace, project and collection selections and typed value operations |

The process-crash tests preserve a committed value across an interrupted
response. Sudden power-loss behavior, long-running production workloads,
released drivers, backup/restore, replication, doctor and
migration cutover remain open.

### Phase 7A Release measurements

The [TCP benchmark example](../../examples/hierarchy_tcp_bench.rs) compares
the historical and hierarchy/1 routes through separate homes on loopback.
Both used synchronous writes and the same server execution-slot setting
(default 8). The runner had five visible Intel Xeon Platinum 8573C CPUs and
an overlay filesystem shared with other work. TLS correctness was tested
separately. These measurements use plaintext loopback and exclude WAN
and TLS overhead.

Four stores each start with two keys and a secondary index. Each worker
authenticates and selects once. After a barrier, it repeats binary GET,
durable binary SET, indexed EXEC query, and begin/SET/commit eight times.
Concentrated clients share a store. Spread clients distribute across four
workspaces. Each writer uses its own key. Throughput counts completed
operations or transactions, with each begin/SET/commit counted once. RSS
and descriptor snapshots include clients and listener threads in the
benchmark process. Percentiles use the nearest rank and are coarse for
small sample counts.

Run with `cargo run --locked --release --example hierarchy_tcp_bench --
<new-scratch-directory> 1 8 32 128`.

| Clients | Workload | Operation | Legacy p50 / p95 / p99 (µs) | Hierarchy/1 p50 / p95 / p99 (µs) |
| ---: | --- | --- | ---: | ---: |
| 1 | concentrated | Authentication | 18,362.70 / 18,362.70 / 18,362.70 | 8,760.51 / 8,760.51 / 8,760.51 |
| 1 | concentrated | Selection | 507.13 / 507.13 / 507.13 | 91.33 / 91.33 / 91.33 |
| 1 | concentrated | GET | 48.52 / 152.18 / 152.18 | 56.29 / 239.92 / 239.92 |
| 1 | concentrated | SET | 402.86 / 631.69 / 631.69 | 461.52 / 682.69 / 682.69 |
| 1 | concentrated | Indexed query | 73.87 / 599.77 / 599.77 | 65.61 / 248.19 / 248.19 |
| 1 | concentrated | Transaction | 520.63 / 775.89 / 775.89 | 544.29 / 920.87 / 920.87 |
| 1 | spread | Authentication | 17,539.52 / 17,539.52 / 17,539.52 | 8,904.96 / 8,904.96 / 8,904.96 |
| 1 | spread | Selection | 92.71 / 92.71 / 92.71 | 76.64 / 76.64 / 76.64 |
| 1 | spread | GET | 43.22 / 110.23 / 110.23 | 51.82 / 134.22 / 134.22 |
| 1 | spread | SET | 424.62 / 612.51 / 612.51 | 401.94 / 1,090.74 / 1,090.74 |
| 1 | spread | Indexed query | 66.35 / 683.19 / 683.19 | 56.63 / 92.26 / 92.26 |
| 1 | spread | Transaction | 442.51 / 587.19 / 587.19 | 506.46 / 3,736.42 / 3,736.42 |
| 8 | concentrated | Authentication | 31,345.44 / 50,624.02 / 50,624.02 | 18,496.54 / 24,230.47 / 24,230.47 |
| 8 | concentrated | Selection | 77.91 / 215.58 / 215.58 | 86.42 / 132.56 / 132.56 |
| 8 | concentrated | GET | 95.51 / 288.08 / 494.54 | 197.15 / 1,081.56 / 1,374.42 |
| 8 | concentrated | SET | 1,390.82 / 2,134.80 / 3,983.48 | 1,444.33 / 3,703.20 / 3,908.91 |
| 8 | concentrated | Indexed query | 120.54 / 433.45 / 515.53 | 188.04 / 1,004.35 / 1,581.80 |
| 8 | concentrated | Transaction | 1,600.32 / 3,803.21 / 3,853.55 | 2,017.99 / 3,976.39 / 5,119.50 |
| 8 | spread | Authentication | 34,921.47 / 59,155.01 / 59,155.01 | 26,007.17 / 37,102.36 / 37,102.36 |
| 8 | spread | Selection | 113.77 / 3,994.13 / 3,994.13 | 403.92 / 4,707.97 / 4,707.97 |
| 8 | spread | GET | 89.13 / 278.93 / 676.52 | 202.81 / 1,270.44 / 2,465.63 |
| 8 | spread | SET | 1,432.96 / 2,297.78 / 2,649.71 | 1,947.63 / 4,968.06 / 10,757.48 |
| 8 | spread | Indexed query | 166.93 / 343.31 / 435.06 | 271.79 / 1,076.07 / 12,153.11 |
| 8 | spread | Transaction | 1,671.06 / 2,495.51 / 3,005.36 | 2,478.64 / 7,201.47 / 22,358.45 |
| 32 | concentrated | Authentication | 92,377.42 / 173,320.37 / 175,453.70 | 40,687.38 / 74,142.68 / 75,510.18 |
| 32 | concentrated | Selection | 236.73 / 28,883.51 / 30,867.71 | 6,840.23 / 38,551.82 / 39,137.51 |
| 32 | concentrated | GET | 146.98 / 625.36 / 801.25 | 758.51 / 4,386.28 / 7,418.29 |
| 32 | concentrated | SET | 2,257.56 / 4,264.89 / 5,708.61 | 2,805.09 / 8,255.30 / 11,616.54 |
| 32 | concentrated | Indexed query | 1,085.41 / 5,496.98 / 7,256.41 | 560.86 / 4,345.08 / 8,335.38 |
| 32 | concentrated | Transaction | 5,307.74 / 9,830.57 / 12,055.16 | 5,844.65 / 13,576.70 / 16,338.46 |
| 32 | spread | Authentication | 95,771.93 / 122,735.07 / 123,874.07 | 49,666.91 / 83,110.19 / 84,435.67 |
| 32 | spread | Selection | 169.91 / 10,881.22 / 28,040.59 | 6,026.61 / 45,958.69 / 64,785.51 |
| 32 | spread | GET | 120.61 / 720.59 / 1,349.26 | 1,672.16 / 9,342.32 / 19,402.53 |
| 32 | spread | SET | 2,674.39 / 6,166.29 / 8,668.81 | 5,134.59 / 11,588.67 / 18,098.46 |
| 32 | spread | Indexed query | 1,128.36 / 5,743.78 / 11,331.60 | 2,004.91 / 9,656.62 / 14,664.38 |
| 32 | spread | Transaction | 5,528.18 / 11,641.09 / 14,882.05 | 9,486.17 / 22,074.42 / 28,527.11 |
| 128 | concentrated | Authentication | 368,910.65 / 575,744.64 / 598,124.49 | 285,228.11 / 542,091.30 / 589,420.19 |
| 128 | concentrated | Selection | 8,052.04 / 56,428.36 / 108,515.76 | 11,057.77 / 315,807.05 / 448,040.56 |
| 128 | concentrated | GET | 138.61 / 768.20 / 2,066.00 | 7,579.76 / 79,027.61 / 117,193.47 |
| 128 | concentrated | SET | 1,980.47 / 4,206.60 / 5,681.30 | 13,955.92 / 78,675.68 / 109,968.11 |
| 128 | concentrated | Indexed query | 6,034.94 / 28,189.56 / 44,590.25 | 8,507.85 / 57,218.19 / 101,815.11 |
| 128 | concentrated | Transaction | 18,479.96 / 47,358.88 / 66,913.73 | 39,215.30 / 112,346.05 / 153,390.85 |
| 128 | spread | Authentication | 317,636.37 / 683,869.92 / 692,476.31 | 160,959.92 / 447,960.39 / 503,941.95 |
| 128 | spread | Selection | 8,235.62 / 55,083.97 / 75,149.91 | 10,705.45 / 326,857.67 / 398,193.97 |
| 128 | spread | GET | 112.05 / 905.26 / 2,472.72 | 7,291.49 / 29,858.09 / 52,507.53 |
| 128 | spread | SET | 2,099.11 / 5,498.15 / 9,734.89 | 9,251.03 / 31,544.96 / 54,327.24 |
| 128 | spread | Indexed query | 7,749.47 / 30,623.06 / 49,624.73 | 6,941.46 / 28,913.43 / 48,669.88 |
| 128 | spread | Transaction | 18,499.80 / 52,084.71 / 73,547.13 | 27,567.73 / 69,603.28 / 93,528.62 |

| Clients | Workload | Operations/s legacy → hierarchy | RSS KiB legacy → hierarchy | Descriptors legacy → hierarchy |
| ---: | --- | ---: | ---: | ---: |
| 1 | concentrated | 3,134 → 3,081 | 7,812 → 12,048 | 18 → 15 |
| 1 | spread | 3,419 → 2,480 | 7,976 → 12,052 | 19 → 18 |
| 8 | concentrated | 8,269 → 5,998 | 8,416 → 12,132 | 40 → 32 |
| 8 | spread | 8,913 → 3,793 | 8,656 → 12,132 | 46 → 32 |
| 32 | concentrated | 11,224 → 8,963 | 9,872 → 12,784 | 121 → 89 |
| 32 | spread | 9,932 → 4,961 | 10,264 → 12,692 | 121 → 89 |
| 128 | concentrated | 13,595 → 4,570 | 14,408 → 15,032 | 409 → 281 |
| 128 | spread | 12,966 → 7,696 | 15,980 → 15,308 | 409 → 281 |

At 128 spread clients the managed mode completed 7,696 operations/s
versus 12,966 on the historical route. Its median GET was 7.29 ms versus
0.11 ms. The managed binary and JSON paths share the execution semaphore
with shell commands. The legacy binary KV frames use their original
admission. Queueing, filesystem load and scheduling all contribute to
this comparison. The run does not isolate their individual costs.

Revocation timings include the acknowledgement of 16 revokes. The managed
check uses the established editor connection. The historical check signs in
again before checking the downgraded grant. The compaction measurement is
a single run. A second client issues 128 reads during that run.

| Maintenance | Legacy p50 / p95 / p99 (µs) | Hierarchy/1 p50 / p95 / p99 (µs) |
| --- | ---: | ---: |
| Revoke acknowledgement | 101.36 / 2,107.47 / 2,107.47 | 710.12 / 4,854.60 / 4,854.60 |
| Compaction | 2,817.24 / 2,817.24 / 2,817.24 | 5,656.50 / 5,656.50 / 5,656.50 |
| GET during compaction run | 33.84 / 76.03 / 579.05 | 42.78 / 139.89 / 245.72 |

The local Phase 6B example was rerun with the Phase 7A code. The following
rows retain its p50/p95/p99 values for the selected metrics. Local numbers
exclude transport, and these runs used separate scratch homes.

| Collections | Open and shutdown (µs) | Selection (µs) | GET (µs) | Compaction (µs) |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 3,691.43 / 5,331.94 / 5,331.94 | 8.16 / 10.41 / 38.87 | 8.64 / 8.98 / 29.66 | 5,623.67 / 6,472.97 / 6,472.97 |
| 32 | 5,974.71 / 7,247.73 / 7,247.73 | 8.09 / 11.51 / 63.26 | 9.08 / 9.37 / 38.18 | 5,138.98 / 8,862.47 / 8,862.47 |
| 128 | 8,288.27 / 11,852.31 / 11,852.31 | 5.18 / 8.07 / 72.32 | 5.64 / 7.08 / 64.75 | 4,786.47 / 8,196.51 / 8,196.51 |
| 1,000 | 89,361.10 / 153,232.72 / 153,232.72 | 8.69 / 63.79 / 122.60 | 9.02 / 12.56 / 146.40 | 7,019.87 / 10,681.81 / 10,681.81 |
| 10,000 | 396,202.28 / 883,187.58 / 883,187.58 | 11.39 / 25.88 / 93.16 | 5.75 / 6.13 / 17.99 | 6,204.11 / 10,769.45 / 10,769.45 |

## Phase 7A.2 native admission

**NOT READY for production.** Native admission changes retain the
authentication, TLS, authorization and durable-write contract of Phase 7A.

The path is socket read and frame validation, payload decode, FIFO lane
admission, shared or exclusive layout guard, account and catalog resolution,
permission check, registry lookup, engine dispatch and synchronous
publication. Encoding and socket write follow dispatch. System account
and policy changes and topology operations retain exclusive admission.
Compaction still blocks data across collections.

The FIFO unit test verifies direct handoff, queue saturation and independent
lanes. The native socket test saturates the data lane and checks the explicit
refusal, continued reads, rollback and revocation. Native end-to-end coverage
retains TLS, slow connections, malformed frames, account and collection
replacement, restart, compaction, transaction pins, lost commit replies and
historical transport isolation. The framing test forces partial writes of the
header and payload.

The seven existing ignored tests remain opt-in workloads.

| Test | Reason |
| --- | --- |
| `hnsw_speed` | Large HNSW comparison reports hardware-dependent speed |
| `hnsw_filter_speed` | Repeats vector searches across filter selectivities for timing |
| `hnsw_sq8_speed` | Measures scalar-quantization performance in Release |
| `query_speed` | Uses a large corpus to compare query timing |
| `group_commit_speed` | Measures disk-dependent batching throughput |
| `invalidation_speed` | Times semantic-cache updates at benchmark scale |
| `managed_routing_component_costs` | Requires a separately provisioned Release benchmark home |

CI still runs the functional assertions for those areas.

### Release diagnosis and comparison

The included [TCP benchmark](../../examples/hierarchy_tcp_bench.rs) was run
on Linux with five visible Intel Xeon Platinum 8573C CPUs, an overlay
filesystem and synchronous writes on both routes. It ran the Phase 7A
reference from `f882b0b` and this branch from the same workspace. The
instrumented reference keeps the shared semaphore and adds only timing
probes. Probe runs are separate from the timed comparisons. System load
and fsync variance affect repeatability on this runner.

#### Phase 7A replay

Each cohort performs the original eight cycles per client. A cycle has
binary GET, durable binary SET, indexed EXEC and a transaction. These are
sample percentiles of that replay, before or after the change. The legacy
column gives the contemporaneous reference throughput.

| Clients | Placement | Legacy ops/s | Shared-slot ops/s | FIFO lanes ops/s | Shared-slot GET p50/p95/p99 (µs) | FIFO lanes GET p50/p95/p99 (µs) |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | concentrated | 2,869.42 | 2,507.44 | 1,112.63 | 64.44 / 320.48 / 320.48 | 83.96 / 423.28 / 423.28 |
| 1 | spread | 2,680.90 | 1,009.42 | 3,164.23 | 74.28 / 213.68 / 213.68 | 33.11 / 40.64 / 40.64 |
| 8 | concentrated | 11,750.72 | 2,469.63 | 7,725.57 | 142.53 / 811.59 / 4,487.08 | 54.24 / 296.18 / 309.17 |
| 8 | spread | 9,075.43 | 2,348.17 | 3,527.87 | 143.19 / 2,471.84 / 2,621.90 | 102.07 / 896.21 / 1,429.33 |
| 32 | concentrated | 15,900.08 | 3,163.42 | 2,855.10 | 919.63 / 12,003.94 / 26,090.81 | 214.70 / 1,020.72 / 2,644.75 |
| 32 | spread | 12,703.14 | 3,881.74 | 3,342.20 | 2,762.48 / 14,743.52 / 41,049.31 | 171.19 / 1,107.24 / 2,844.28 |
| 128 | concentrated | 13,116.69 | 3,711.21 | 3,134.35 | 7,654.48 / 61,237.83 / 99,144.05 | 167.50 / 1,812.61 / 4,897.48 |
| 128 | spread | 13,603.16 | 5,024.01 | 4,538.13 | 10,112.87 / 47,408.88 / 95,697.61 | 157.96 / 1,198.94 / 3,216.87 |

At 128 spread clients the paired replay reduced GET p50 from 10.11 ms
to 0.158 ms and p99 from 95.70 ms to 3.22 ms. Replay throughput fell from
5,024 to 4,538 operations/s. The sustained mixed workload below improved
from 3,356 to 3,875 operations/s at that placement. The all-read, indexed
and transaction-only profiles lost throughput there. A single global
speed-up would misrepresent these results.

At 128 clients, the baseline instrumented replay attributes most GET
latency to slot wait. The baseline GET averages were 6,540.64 µs in the
slot queue and 16.59 µs in execution. SET spent 7,650.98 µs in the queue
and 1,316.31 µs in group sync for the operations that synced. GET account
resolution averaged 57.08 µs and registry acquisition averaged 13.15 µs.
These are wall-clock means under load. They include scheduling delays.
The frame-read timer includes time waiting for the next client frame and
cannot be added to the request latency. Stage times overlap where execution
includes group sync or group sync includes disk time.

| Operation | Stage | Baseline samples | Baseline mean (µs) | Optimized samples | Optimized mean (µs) |
| --- | --- | ---: | ---: | ---: | ---: |
| auth | authentication | 274 | 21,290.32 | 274 | 21,924.51 |
| auth | frame_read | 274 | 1,350.37 | 274 | 1,278.42 |
| auth | frame_write | 274 | 407.87 | 274 | 138.50 |
| auth | layout | 274 | 0.23 | 274 | 0.27 |
| auth | queue | 274 | 126,731.94 | 274 | 341,365.97 |
| exec | account | 6451 | 49.24 | 6454 | 76.73 |
| exec | collection | 6145 | 0.67 | 6146 | 0.53 |
| exec | decode | 6451 | 4.34 | 6454 | 2.98 |
| exec | disk | — | — | 841 | 991.54 |
| exec | execute | 6145 | 477.89 | 6146 | 707.01 |
| exec | frame_read | 6451 | 231.02 | 6454 | 298.44 |
| exec | frame_write | 6451 | 28.76 | 6454 | 33.18 |
| exec | layout | 6451 | 0.17 | 6454 | 0.55 |
| exec | permission | 6145 | 12.07 | 6146 | 21.47 |
| exec | queue | 6451 | 9,356.57 | 6454 | 14,004.92 |
| exec | registry | 6145 | 2.25 | 6146 | 2.19 |
| exec | sync | 2048 | 1,329.97 | 2048 | 2,004.45 |
| get | account | 2176 | 57.08 | 2304 | 74.58 |
| get | collection | 2176 | 0.81 | 2304 | 0.98 |
| get | decode | — | — | 2304 | 0.58 |
| get | encode | 2176 | 0.78 | 2304 | 0.48 |
| get | execute | 2176 | 16.59 | 2304 | 18.93 |
| get | frame_read | 2176 | 13,780.56 | 2304 | 45,767.71 |
| get | frame_write | 2176 | 25.31 | 2304 | 25.18 |
| get | layout | 2176 | 3.28 | 2304 | 13.08 |
| get | permission | 2176 | 14.23 | 2304 | 19.46 |
| get | queue | 2176 | 6,540.64 | 2304 | 221.16 |
| get | registry | 2176 | 13.15 | 2304 | 10.54 |
| set | account | 4112 | 46.09 | 4112 | 65.68 |
| set | collection | 4112 | 1.22 | 4112 | 1.05 |
| set | decode | 4096 | 1.58 | 4096 | 1.29 |
| set | disk | — | — | 846 | 969.64 |
| set | execute | 4096 | 676.28 | 4096 | 949.94 |
| set | frame_read | 4112 | 110.49 | 4112 | 160.32 |
| set | frame_write | 4112 | 28.04 | 4112 | 30.50 |
| set | layout | 4112 | 0.12 | 4112 | 0.52 |
| set | permission | 4112 | 13.23 | 4112 | 14.24 |
| set | queue | 4112 | 7,650.98 | 4112 | 16,314.84 |
| set | registry | 4096 | 2.78 | 4096 | 4.94 |
| set | sync | 2048 | 1,316.31 | 2048 | 1,856.82 |

#### Slot sweep

Each row uses the original 128-client spread workload with the indicated
`server.threads`. The baseline shares the slots. This branch applies the
same setting independently to reads and data work and reserves control and
authentication admission. This table measures the routes as implemented,
including those capacity differences.

| Slots | Shared-slot ops/s | FIFO lanes ops/s | Shared-slot GET p50/p95/p99 (µs) | FIFO lanes GET p50/p95/p99 (µs) |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 705.50 | 613.06 | 83,040.24 / 347,851.76 / 619,689.45 | 119.28 / 11,813.89 / 12,993.86 |
| 4 | 3,631.45 | 1,850.01 | 13,037.62 / 63,911.92 / 94,028.09 | 162.22 / 1,797.81 / 8,211.94 |
| 8 | 8,396.95 | 2,838.22 | 6,736.28 / 27,010.30 / 38,961.96 | 213.32 / 26,916.92 / 28,106.66 |
| 16 | 11,506.32 | 7,408.47 | 4,711.91 / 18,240.02 / 26,833.42 | 264.47 / 5,107.19 / 8,304.55 |
| 32 | 14,791.80 | 9,784.99 | 3,042.17 / 12,226.47 / 18,661.97 | 284.28 / 3,586.34 / 11,027.89 |
| 64 | 12,625.73 | 12,489.39 | 3,164.57 / 14,671.68 / 20,130.73 | 983.53 / 8,270.36 / 15,403.44 |

#### Sustained profiles and TLS

Each store contains 512 seeded 256-byte keys plus an indexed document.
Clients warm up for 150 ms, then synchronize and measure for 1 second.
Each row summarizes two independent scratch-home runs. Values are the
median of the two run-level measurements. Percentiles are summarized by
run and do not pool individual samples. The benchmark reports counts of
completed operations and stopped workers. A failed write is counted and
is never retried by the harness. CPU means Linux process user and system
time, for both server and clients, divided by wall time. RSS and open
descriptors are sampled after warmup and before load starts. The TLS cases
pin the localhost server certificate. There was no remote WAN test. The
existing 128-socket ceiling excluded 256-client runs.

Use `KIVADB_BENCH_PROFILE=mixed KIVADB_BENCH_MS=1000` with
`cargo run --release --example hierarchy_tcp_bench -- <new-home> 1 8 32 128`.
Other profiles are `read`, `write`, `95read`, `50read`, `index` and `txn`.
`KIVADB_BENCH_TLS=1` enables loopback TLS for profiled runs.
`KIVADB_BENCH_SLOTS` overrides the slot count for a new benchmark home.
`KIVADB_BENCH_NATIVE_METRICS=1` enables stage probes.

| Clients | Benchmark | Placement | Ops/s shared → FIFO lanes | CPU cores shared → FIFO lanes | RSS KiB shared → FIFO lanes | FDs shared → FIFO lanes | Error rate shared → FIFO lanes (%) |
| ---: | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 128 | read | concentrated | 21,490.26 → 14,923.67 | 2.02 → 1.47 | 19,472.00 → 20,010.00 | 271.00 → 271.00 | 0.00 → 0.00 |
| 128 | read | spread | 23,110.34 → 16,887.84 | 2.05 → 1.65 | 19,802.00 → 20,282.00 | 277.00 → 277.00 | 0.00 → 0.00 |
| 128 | write | concentrated | 1,592.29 → 2,563.01 | 0.41 → 0.55 | 18,912.00 → 18,206.00 | 272.00 → 272.00 | 0.00 → 0.00 |
| 128 | write | spread | 1,602.20 → 2,560.42 | 0.48 → 0.51 | 19,088.00 → 18,412.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 128 | 95read | concentrated | 15,043.26 → 18,576.22 | 1.52 → 1.93 | 20,848.00 → 21,630.00 | 272.00 → 272.00 | 0.00 → 0.00 |
| 128 | 95read | spread | 14,563.22 → 15,022.42 | 1.65 → 1.75 | 20,894.00 → 22,010.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 128 | 50read | concentrated | 1,903.49 → 5,516.33 | 0.51 → 0.70 | 19,950.00 → 19,050.00 | 272.00 → 272.00 | 0.00 → 0.00 |
| 128 | 50read | spread | 1,595.14 → 5,802.61 | 0.59 → 0.84 | 20,274.00 → 19,054.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 128 | index | concentrated | 16,022.15 → 16,553.85 | 1.81 → 1.85 | 19,420.00 → 19,384.00 | 271.00 → 271.00 | 0.00 → 0.00 |
| 128 | index | spread | 18,461.82 → 16,421.31 | 2.02 → 1.81 | 19,548.00 → 19,664.00 | 277.00 → 277.00 | 0.00 → 0.00 |
| 128 | txn | concentrated | 1,024.69 → 1,128.22 | 0.59 → 0.51 | 18,312.00 → 17,900.00 | 272.00 → 272.00 | 0.00 → 0.00 |
| 128 | txn | spread | 1,611.72 → 1,393.66 | 0.94 → 0.69 | 18,678.00 → 18,398.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 1 | mixed | concentrated | 1,192.83 → 397.78 | 0.21 → 0.08 | 15,156.00 → 15,754.00 | 18.00 → 18.00 | 0.00 → 0.00 |
| 1 | mixed | spread | 1,614.21 → 1,136.23 | 0.20 → 0.19 | 15,164.00 → 15,770.00 | 18.00 → 18.00 | 0.00 → 0.00 |
| 8 | mixed | concentrated | 5,134.05 → 4,265.48 | 0.83 → 0.55 | 15,300.00 → 15,906.00 | 32.00 → 32.00 | 0.00 → 0.00 |
| 8 | mixed | spread | 4,925.61 → 1,822.39 | 0.85 → 0.42 | 15,426.00 → 15,740.00 | 41.00 → 41.00 | 0.00 → 0.00 |
| 32 | mixed | concentrated | 4,704.09 → 851.82 | 0.92 → 0.25 | 16,322.00 → 16,588.00 | 89.00 → 89.00 | 0.00 → 0.00 |
| 32 | mixed | spread | 4,675.74 → 2,385.71 | 1.06 → 0.66 | 16,194.00 → 16,452.00 | 89.00 → 89.00 | 0.00 → 0.00 |
| 128 | mixed | concentrated | 4,177.66 → 4,795.74 | 1.04 → 0.93 | 19,348.00 → 19,612.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 128 | mixed | spread | 3,356.39 → 3,874.74 | 1.12 → 0.81 | 19,706.00 → 20,398.00 | 281.00 → 281.00 | 0.00 → 0.00 |
| 8 | tls | concentrated | 2,499.79 → 4,513.84 | 0.78 → 1.25 | 19,324.00 → 19,802.00 | 112.00 → 112.00 | 0.00 → 0.00 |
| 8 | tls | spread | 2,780.58 → 3,438.86 | 0.90 → 1.22 | 18,768.00 → 18,944.00 | 121.00 → 121.00 | 0.00 → 0.00 |
| 32 | tls | concentrated | 3,115.57 → 5,740.76 | 1.12 → 1.80 | 20,834.00 → 21,040.00 | 409.00 → 409.00 | 0.00 → 0.00 |
| 32 | tls | spread | 3,072.43 → 4,575.24 | 1.17 → 1.82 | 21,062.00 → 21,090.00 | 409.00 → 409.00 | 0.00 → 0.00 |

The following latency comparison includes all selected profile operations.
Units are microseconds, shown as p50/p95/p99.

| Clients | Benchmark | Placement | Operation | Shared slots | FIFO lanes |
| ---: | --- | --- | --- | ---: | ---: |
| 128 | read | concentrated | get | 2,868.19 / 24,170.68 / 38,014.44 | 4,851.12 / 28,658.85 / 39,253.84 |
| 128 | read | spread | get | 2,682.07 / 20,551.62 / 34,245.20 | 4,472.46 / 21,060.71 / 57,699.18 |
| 128 | write | concentrated | set | 77,399.71 / 212,090.14 / 310,263.58 | 67,553.24 / 104,224.41 / 113,555.28 |
| 128 | write | spread | set | 78,505.92 / 207,847.25 / 297,823.79 | 71,979.65 / 105,075.10 / 111,487.34 |
| 128 | 95read | concentrated | get | 3,612.28 / 33,370.11 / 51,635.04 | 4,115.77 / 16,669.58 / 28,281.74 |
| 128 | 95read | concentrated | set | 9,718.58 / 42,966.42 / 64,720.93 | 37,801.68 / 54,710.11 / 60,800.12 |
| 128 | 95read | spread | get | 3,863.01 / 33,049.06 / 51,123.83 | 4,371.07 / 19,312.38 / 31,791.25 |
| 128 | 95read | spread | set | 9,221.11 / 40,302.12 / 56,917.78 | 42,433.47 / 69,685.14 / 91,482.09 |
| 128 | 50read | concentrated | get | 39,493.12 / 187,588.66 / 286,591.92 | 231.20 / 2,485.02 / 4,908.44 |
| 128 | 50read | concentrated | set | 54,630.26 / 204,089.44 / 313,858.32 | 78,908.89 / 110,752.78 / 116,343.35 |
| 128 | 50read | spread | get | 55,333.79 / 262,268.57 / 413,723.92 | 191.87 / 1,967.50 / 3,614.98 |
| 128 | 50read | spread | set | 74,341.18 / 278,742.69 / 431,054.25 | 63,857.09 / 81,073.96 / 87,601.26 |
| 128 | index | concentrated | indexed | 3,458.37 / 29,894.60 / 48,662.46 | 5,486.88 / 20,826.48 / 26,263.49 |
| 128 | index | spread | indexed | 3,023.11 / 28,853.24 / 46,993.84 | 5,459.24 / 21,684.77 / 31,657.25 |
| 128 | txn | concentrated | transaction | 101,791.16 / 255,198.99 / 348,698.84 | 137,153.03 / 230,547.53 / 236,276.75 |
| 128 | txn | spread | transaction | 66,649.92 / 184,819.80 / 254,190.91 | 86,234.23 / 233,583.57 / 247,763.10 |
| 1 | mixed | concentrated | auth | 12,421.16 / 12,421.16 / 12,421.16 | 11,944.53 / 11,944.53 / 11,944.53 |
| 1 | mixed | concentrated | selection | 2,639.58 / 2,639.58 / 2,639.58 | 1,814.14 / 1,814.14 / 1,814.14 |
| 1 | mixed | concentrated | get | 100.93 / 296.24 / 2,557.36 | 73.03 / 3,047.06 / 6,951.49 |
| 1 | mixed | concentrated | set | 2,135.74 / 4,874.66 / 16,541.07 | 2,800.22 / 17,753.99 / 26,285.01 |
| 1 | mixed | concentrated | indexed | 120.44 / 382.35 / 12,440.32 | 103.29 / 1,979.93 / 5,954.46 |
| 1 | mixed | concentrated | transaction | 2,285.84 / 13,633.70 / 19,704.75 | 3,355.20 / 16,694.15 / 36,863.72 |
| 1 | mixed | spread | auth | 14,198.42 / 14,198.42 / 14,198.42 | 17,501.14 / 17,501.14 / 17,501.14 |
| 1 | mixed | spread | selection | 4,177.82 / 4,177.82 / 4,177.82 | 135.82 / 135.82 / 135.82 |
| 1 | mixed | spread | get | 92.20 / 240.35 / 417.65 | 75.53 / 1,270.18 / 4,594.35 |
| 1 | mixed | spread | set | 2,119.23 / 10,136.60 / 14,100.13 | 908.12 / 6,597.31 / 14,939.05 |
| 1 | mixed | spread | indexed | 116.75 / 272.59 / 2,666.82 | 97.78 / 1,308.93 / 2,572.57 |
| 1 | mixed | spread | transaction | 2,176.98 / 5,992.51 / 14,528.31 | 889.71 / 12,197.16 / 16,962.74 |
| 8 | mixed | concentrated | auth | 22,591.84 / 49,052.33 / 49,052.33 | 54,620.84 / 87,245.36 / 87,245.36 |
| 8 | mixed | concentrated | selection | 823.72 / 18,484.61 / 18,484.61 | 960.16 / 7,170.45 / 7,170.45 |
| 8 | mixed | concentrated | get | 167.77 / 2,062.27 / 5,261.25 | 102.78 / 2,299.41 / 13,982.05 |
| 8 | mixed | concentrated | set | 4,094.52 / 16,471.24 / 21,039.19 | 3,374.18 / 28,800.74 / 47,845.51 |
| 8 | mixed | concentrated | indexed | 199.81 / 3,205.57 / 12,078.70 | 149.59 / 2,457.39 / 10,962.31 |
| 8 | mixed | concentrated | transaction | 4,647.44 / 16,731.17 / 23,008.60 | 3,816.68 / 38,936.67 / 52,863.28 |
| 8 | mixed | spread | auth | 22,318.65 / 29,936.96 / 29,936.96 | 78,324.76 / 96,757.73 / 96,757.73 |
| 8 | mixed | spread | selection | 1,559.70 / 11,018.21 / 11,018.21 | 2,068.59 / 6,914.65 / 6,914.65 |
| 8 | mixed | spread | get | 147.09 / 1,883.02 / 3,117.89 | 151.47 / 2,642.41 / 5,267.93 |
| 8 | mixed | spread | set | 2,639.45 / 10,321.47 / 19,213.35 | 3,651.22 / 24,892.27 / 66,765.13 |
| 8 | mixed | spread | indexed | 175.37 / 1,839.44 / 3,199.15 | 193.00 / 3,262.81 / 18,164.23 |
| 8 | mixed | spread | transaction | 3,154.53 / 14,048.10 / 20,842.36 | 4,595.84 / 43,135.92 / 116,055.26 |
| 32 | mixed | concentrated | auth | 77,779.32 / 139,620.20 / 143,177.51 | 160,656.40 / 275,349.34 / 297,875.20 |
| 32 | mixed | concentrated | selection | 13,638.21 / 84,786.55 / 99,321.43 | 1,555.54 / 26,787.38 / 31,191.91 |
| 32 | mixed | concentrated | get | 1,613.60 / 17,384.97 / 26,419.49 | 227.82 / 6,373.56 / 22,152.76 |
| 32 | mixed | concentrated | set | 7,060.42 / 24,438.07 / 37,678.67 | 29,040.79 / 112,115.33 / 159,539.64 |
| 32 | mixed | concentrated | indexed | 1,750.23 / 17,437.53 / 27,032.63 | 10,656.63 / 90,198.17 / 136,366.05 |
| 32 | mixed | concentrated | transaction | 17,420.11 / 42,435.67 / 52,366.84 | 95,748.21 / 205,479.42 / 263,636.72 |
| 32 | mixed | spread | auth | 62,179.06 / 117,481.26 / 136,367.94 | 162,475.94 / 289,758.58 / 291,520.45 |
| 32 | mixed | spread | selection | 17,739.19 / 63,035.83 / 73,326.05 | 1,166.26 / 10,464.34 / 15,346.51 |
| 32 | mixed | spread | get | 2,477.89 / 16,579.44 / 25,512.67 | 203.00 / 2,581.67 / 10,973.84 |
| 32 | mixed | spread | set | 6,579.97 / 23,461.16 / 31,714.10 | 9,773.92 / 46,256.68 / 64,382.93 |
| 32 | mixed | spread | indexed | 2,392.82 / 16,722.52 / 27,202.40 | 5,230.47 / 32,508.47 / 46,889.62 |
| 32 | mixed | spread | transaction | 14,954.60 / 38,190.12 / 52,041.21 | 21,024.12 / 74,519.20 / 98,460.33 |
| 128 | mixed | concentrated | auth | 243,507.01 / 589,652.77 / 629,658.74 | 544,309.84 / 1,060,673.19 / 1,079,990.41 |
| 128 | mixed | concentrated | selection | 93,570.11 / 361,729.09 / 524,088.42 | 972.29 / 10,537.12 / 39,664.15 |
| 128 | mixed | concentrated | get | 15,299.23 / 84,076.56 / 132,061.55 | 182.17 / 1,315.64 / 3,058.53 |
| 128 | mixed | concentrated | set | 23,492.66 / 83,647.02 / 135,594.71 | 27,906.92 / 53,700.81 / 59,347.32 |
| 128 | mixed | concentrated | indexed | 14,757.35 / 79,172.88 / 126,934.09 | 20,952.87 / 47,079.86 / 54,900.65 |
| 128 | mixed | concentrated | transaction | 72,395.98 / 201,263.13 / 262,468.03 | 59,994.44 / 101,260.14 / 118,020.72 |
| 128 | mixed | spread | auth | 261,373.75 / 566,782.97 / 610,253.78 | 548,957.25 / 1,044,852.25 / 1,075,115.54 |
| 128 | mixed | spread | selection | 84,511.71 / 362,818.09 / 414,563.93 | 1,325.04 / 15,288.08 / 19,482.74 |
| 128 | mixed | spread | get | 14,596.04 / 98,313.98 / 156,424.68 | 167.09 / 1,391.66 / 3,843.30 |
| 128 | mixed | spread | set | 20,594.09 / 92,074.35 / 146,922.89 | 33,997.22 / 56,941.42 / 131,538.33 |
| 128 | mixed | spread | indexed | 14,990.02 / 90,791.21 / 140,428.29 | 22,996.50 / 45,951.49 / 96,782.24 |
| 128 | mixed | spread | transaction | 71,261.24 / 200,308.88 / 255,805.54 | 61,687.58 / 150,919.83 / 175,984.89 |
| 8 | tls | concentrated | auth | 22,954.15 / 36,230.87 / 36,230.87 | 26,920.89 / 40,259.26 / 40,259.26 |
| 8 | tls | concentrated | selection | 811.32 / 8,407.76 / 8,407.76 | 342.17 / 8,245.66 / 8,245.66 |
| 8 | tls | concentrated | get | 430.24 / 2,787.07 / 11,807.02 | 318.88 / 1,902.95 / 3,262.41 |
| 8 | tls | concentrated | set | 3,608.17 / 15,033.05 / 26,686.79 | 2,814.45 / 14,925.44 / 17,839.37 |
| 8 | tls | concentrated | indexed | 449.32 / 2,867.43 / 14,191.76 | 346.23 / 1,988.19 / 3,537.95 |
| 8 | tls | concentrated | transaction | 4,600.61 / 16,447.73 / 27,261.10 | 3,510.39 / 14,308.66 / 19,101.38 |
| 8 | tls | spread | auth | 38,403.45 / 71,661.89 / 71,661.89 | 39,895.50 / 51,037.85 / 51,037.85 |
| 8 | tls | spread | selection | 2,350.44 / 23,775.77 / 23,775.77 | 1,690.65 / 18,837.10 / 18,837.10 |
| 8 | tls | spread | get | 473.75 / 3,083.07 / 10,248.10 | 416.94 / 2,401.04 / 9,803.70 |
| 8 | tls | spread | set | 2,880.03 / 10,367.74 / 22,840.74 | 2,685.10 / 6,910.89 / 17,453.65 |
| 8 | tls | spread | indexed | 504.07 / 3,540.44 / 17,034.28 | 455.34 / 2,687.60 / 11,496.91 |
| 8 | tls | spread | transaction | 4,398.48 / 16,533.47 / 29,128.63 | 3,886.98 / 14,443.15 / 22,915.23 |
| 32 | tls | concentrated | auth | 78,038.14 / 119,224.20 / 128,543.80 | 71,400.07 / 131,185.37 / 133,622.57 |
| 32 | tls | concentrated | selection | 18,462.30 / 68,953.27 / 72,940.07 | 1,416.55 / 12,187.16 / 15,111.33 |
| 32 | tls | concentrated | get | 3,878.64 / 21,540.23 / 31,170.20 | 413.65 / 1,431.59 / 7,350.74 |
| 32 | tls | concentrated | set | 9,613.24 / 29,452.51 / 38,717.88 | 5,006.01 / 12,772.31 / 17,514.10 |
| 32 | tls | concentrated | indexed | 4,277.23 / 19,995.20 / 31,018.08 | 2,958.64 / 5,910.73 / 14,906.48 |
| 32 | tls | concentrated | transaction | 23,486.47 / 50,663.16 / 61,049.29 | 11,130.22 / 21,459.35 / 25,252.77 |
| 32 | tls | spread | auth | 62,632.40 / 123,805.99 / 152,917.27 | 107,966.41 / 187,420.44 / 205,350.71 |
| 32 | tls | spread | selection | 22,646.40 / 81,923.35 / 88,022.76 | 1,824.26 / 10,978.26 / 21,763.53 |
| 32 | tls | spread | get | 3,883.38 / 18,837.49 / 29,309.33 | 548.33 / 2,519.91 / 8,804.92 |
| 32 | tls | spread | set | 8,102.47 / 27,776.58 / 38,370.21 | 5,986.54 / 17,472.38 / 25,004.13 |
| 32 | tls | spread | indexed | 3,731.03 / 20,802.50 / 28,722.39 | 3,608.80 / 8,422.62 / 19,593.79 |
| 32 | tls | spread | transaction | 19,229.85 / 43,833.64 / 55,413.56 | 13,049.27 / 28,621.37 / 48,355.27 |

#### Revocation and compaction

The 128-client mixed profiles also time 16 grant revocation replies plus
one compaction with a concurrent reader in the same collection and one
with a reader in another workspace. These are post-load maintenance
tests. A median can conceal the single stall while the layout barrier is
held. The isolated compaction command still takes the instance barrier.

| Metric | Shared-slot p50/p95/p99 (µs) | FIFO lanes p50/p95/p99 (µs) |
| --- | ---: | ---: |
| revoke | 726.98 / 1,399.38 / 1,399.38 | 1,378.64 / 4,703.84 / 4,703.84 |
| compact-same | 34,283.74 / 34,283.74 / 34,283.74 | 24,948.57 / 24,948.57 / 24,948.57 |
| get-during-compact-same | 35.33 / 396.69 / 1,229.94 | 55.80 / 155.25 / 3,544.54 |
| compact-other | 6,460.60 / 6,460.60 / 6,460.60 | 35,660.96 / 35,660.96 / 35,660.96 |
| get-during-compact-other | 35.40 / 144.15 / 265.09 | 56.62 / 203.57 / 4,168.41 |

The local Release example rerun checks the shared routing path without TCP.

| Collections | GET p50/p95/p99 (µs) | Startup and shutdown p50/p95/p99 (µs) |
| ---: | ---: | ---: |
| 1 | 7.89 / 8.76 / 50.66 | 3,391.36 / 4,342.77 / 4,342.77 |
| 32 | 5.61 / 8.03 / 19.60 | 3,845.47 / 4,640.84 / 4,640.84 |
| 128 | 5.58 / 8.21 / 192.18 | 7,152.99 / 9,060.31 / 9,060.31 |
| 1,000 | 5.71 / 6.16 / 34.38 | 59,774.71 / 80,152.13 / 80,152.13 |

## Phase 7B REST and RESP

**NOT READY for production.** The [adapter guide](../architecture/HIERARCHICAL_REST_RESP.md)
covers startup, authorization, compatibility and operational limits.

| Coverage | Regression checks |
| --- | --- |
| REST01–REST07 | Basic authentication, collection creation, rename and retirement, explicit cascade, default protection, homonyms, typed binary values, Viewer restrictions, indistinguishable missing and hidden resources, and revoked access |
| REST08–REST12 | Invalid escape, duplicate framing headers, oversized body, pinned TLS, concurrent writes from separate transports, and a rolled-back batch |
| RESP01–RESP10, RESP12 | AUTH, explicit selection, isolation, GET/SET/DEL, TTL, MULTI/EXEC, pipelines, revoked grants, malformed frames, and disconnect before EXEC |
| RESP11 | Shared connection cap, queue capacity, lane isolation and removal of expired waiters |
| CROSS01–CROSS06 | Shared data, grant revocation during an open RESP transaction, stale identity after re-creation, and persistence after restart |

The historical compatibility suites still run in the default test suite.
The existing seven ignored performance workloads remain opt-in. External
driver adaptation, push, backup, restore, replication, doctor, reattachment
of retired resources and final migration remain future work.
