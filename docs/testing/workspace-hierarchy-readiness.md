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
