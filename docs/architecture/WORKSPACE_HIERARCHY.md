# Workspace hierarchy

## Current implementation

The baseline is `cd2d30a` on `main`. The instance maps a storage name to an
engine through `Config::workspace_dir`. The special storage name `default`
points to the root data store. Other storage paths come from `[workspaces]`
in `kiva.conf`. Projects are `project:*` records in the system engine.
Project membership is inferred from the first segment of a storage name.
The alias added in #160 translates `default.project.collection` in the shell.

| Area | Current dependency | Required change |
| --- | --- | --- |
| Instance and configuration | Storage name keys the engine map and selects a path | Resolve catalog IDs before opening engines |
| Session, CLI and completion | `Session::ws` selects both an engine and an authorization target | Track workspace, project and collection independently |
| Authorization | Historical grants match storage names and project prefixes | Explicit resource scopes and exact migration of existing grants |
| Storage | Data, metadata and journals are attached to engine paths | Preserve existing files and allocate fresh storage for new IDs |
| Native clients, REST and Redis | `server.rs`, `api.rs`, `http.rs` and `resp.rs` select stores by name or index | Keep compatibility translation at the protocol boundary |
| Drivers and scripts | Python and Node.js send native commands and requests, while Lua and KVA use session context | Expose qualified collections and document command changes |
| Backup and restore | The manifest indexes stores by name and supports restoring one store | Version the manifest and preserve parent identities |
| Replication and events | Replication streams and subscriptions route by storage name | Keep routing distinct across workspaces and stable across renames |
| Doctor and recovery | Doctor enumerates config paths, while instance startup replays pending project, collection and move records | Validate catalog relations and recover catalog and filesystem changes together |

## Target model

A versioned system catalog records persistent IDs for workspaces, projects and
collections. Each project has exactly one workspace parent. Each collection
has exactly one project parent and owns a separate engine. An empty workspace
creates no user store. The default workspace always exists and cannot be
renamed or removed. Names compare case-insensitively within each type and
parent, using ASCII rules on every platform. A rename changes the display name
while retaining the ID.

The same resolver serves administration and navigation. A separate translation
layer keeps historical names available to existing clients, subject to the
same access checks as their canonical targets. Session navigation validates
all parents and authorization before changing context. A transaction stays
bound to its collection. A drop unregisters a resource and retains the files.
A new collection with the same name must receive a fresh storage identity.

## Migration and recovery plan

1. Inspect the historical source read-only and validate paths, records and
   pending operations. Verify a uniquely named backup before activation.
2. Import projects into `default`. Import their collections under the same
   projects. Import standalone stores under `default.legacy` only when needed.
   If that name is occupied, select the first available case-insensitive name
   from `legacy_2`, `legacy_3` and so on. Record the mapping for clients.
3. Preserve paths to old engines, indexes, encrypted files and journals.
   Validate canonical paths and reject shared or missing stores. Translate
   historical grants using an explicitly identified legacy scope so a new
   hierarchy wildcard cannot broaden them.
4. Publish the catalog and mapping only after the source and backup validate.
   Use one durable catalog update for metadata and recoverable intent records
   for operations that also allocate a store. Verify an interrupted activation
   and a repeated migration against the 1.7.1 and release-candidate fixtures.

The existing backup and replication formats route by storage name. Activating
hierarchical writes before those paths can preserve the required IDs and
resolve access correctly would allow restored and replicated state to diverge.
Hierarchical routing stays inactive until the migration and dependent access paths are integrated.

## Catalog storage implemented in this branch

The catalog store publishes the complete versioned snapshot as one record in
the system engine. Each update starts an optimistic engine transaction and
loads the current snapshot. The engine checks for a conflicting write before
committing. The store syncs the engine before returning success. Failed
validation leaves the stored snapshot untouched. The store distinguishes
absence, unsupported versions and malformed records. Startup validates a staged record before account migration and administrative recovery. Historical homes may still omit this record. A staged record never changes storage routing.


## Pull request sequence

1. **Startup and migration preflight.** Load and validate the staged record,
   resolve a collection identity within its parents, and preview existing
   collection names and physical paths. Rehearse reopening staged catalogs on
   copied historical homes. The preview is advisory and does not write.
2. **Offline migration and rollback.** Validate legacy records and grants
   while catalog mutations are quiesced. Verify a unique backup and exercise
   durable migration, recovery from interruption, and rollback. Keep the
   staged hierarchy separate from serving.
3. **Sessions and administration.** Add typed context and explicit naming,
   then enforce workspace and collection scope and revocation before allowing
   hierarchical data access. Exercise concurrent navigation and mutations.
4. **Interfaces and maintenance.** Adapt native TCP, REST, Redis and both
   drivers, along with backup, restore, replication and doctor. Test historical
   aliases with scoped permissions before enabling new routing by default.
5. **Activation and validation.** Enable serving only with compatible storage
   and permissions, complete crash and migration checks, and measure the
   hierarchy against the release baseline.
