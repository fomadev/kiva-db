# Workspace hierarchy readiness

**NOT READY**

This branch contains the architecture audit, a versioned catalog model, and a
read-only planner for historical names, and a catalog store backed by the system engine. The instance still starts and serves
through the existing storage registration. The branch does not migrate any
home or change any command.

| Area | Status | Evidence or remaining work |
| --- | --- | --- |
| Architecture before and target | PASS | [Architecture audit and migration plan](../architecture/WORKSPACE_HIERARCHY.md) |
| Model validation | PASS | Catalog tests cover typed IDs, mandatory parents, case-insensitive uniqueness, distinct homonyms and malformed snapshots |
| Migration mapping | PASS | Tests cover standalone stores, an occupied `legacy` name and refusal of orphaned or conflicting input |
| Catalog storage | PASS | The store writes one versioned system record. Tests cover reopen, rollback on validation errors and rejection of a stale writer |
| Crash recovery | NON TESTÉ | The store reuses the engine transaction format. Crash tests spanning catalog and collection files are outstanding |
| Data migration and rollback | NON TESTÉ | Existing stores are not written. Backup verification, path validation, authorization migration and activation are outstanding |
| Engine, CLI and network integration | NON TESTÉ | Session context, CRUD, navigation, compatibility protocol, Rust API integration and drivers are outstanding |
| Security and concurrency | NON TESTÉ | The model validates parent relationships. Cross-workspace authorization, revocation and concurrent administrative operations are outstanding |
| Backup, replication, and doctor | NON TESTÉ | Remain bound to the existing storage names |
| Historical fixture preview | PASS | Mapping checked on copies of 1.7.1, encrypted 1.7.1, and release candidates 2, 3 and 4 |
| Benchmarks and migrated fixtures | NON TESTÉ | No end-to-end migration or scale comparison has been run |
| Multiplatform CI | NON TESTÉ | Verify this branch's CI before evaluating platform coverage |

## Changes and verification

Added `src/catalog.rs`, `tests/workspace_hierarchy.rs`, the architecture
document and this report. Updated `src/lib.rs`, `Cargo.toml`, `Cargo.lock` and `tests/compat.rs`.
The serde version already in the lockfile is reused with derive support.
No files or commands were removed. The changes introduce no command
incompatibilities in the running application.

Run `cargo test --locked --test workspace_hierarchy` to verify the model.
No production migration procedure is available. Before this feature can be
enabled, verify the migration and rollback on isolated copies of supported
homes and run the concurrency, crash recovery and cross-interface tests
specified in the architecture work.
