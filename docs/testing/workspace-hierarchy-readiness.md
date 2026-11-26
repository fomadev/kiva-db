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
REST, RESP, released drivers, backup/restore, replication, doctor and
migration cutover remain open.
