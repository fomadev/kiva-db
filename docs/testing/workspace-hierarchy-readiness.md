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
