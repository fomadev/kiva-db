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

## Offline staging and rollback

Stop the instance and quiesce other writers to any custom store paths.
Run these commands with the same key configuration used for serving
encrypted homes:

```sh
kivadb migrate plan --home /path/to/home
kivadb migrate apply --home /path/to/home
kivadb migrate status --home /path/to/home
kivadb migrate verify --home /path/to/home
kivadb migrate rollback --home /path/to/home
```

The commands take the home lock. Planning and applying also take the normal
engine locks on the registered stores. Planning validates raw logs and opens
private scratch copies to detect records that cannot be read or need
recovery. Missing storage, pending administration, overlapping paths,
directory or file symlinks encountered in the source tree, and duplicate
physical store identities prevent staging. The preview creates no backup or
catalog metadata.

Apply copies the configuration, system files, registered store directories
(including indexes, schema metadata and change logs), and any present TLS
files and audit log into a unique `workspace-migration-backups/<id>`
directory. The manifest associates each numbered file with its original
path and SHA-256 digest. Preserve this directory, plus any external
encryption keys and runtime configuration. Lock files are transient and
excluded. The backup protects file contents and paths. It does not preserve
filesystem ACLs or serve as an input to `kivadb restore`.

After verifying the copied bytes, apply publishes the versioned catalog,
alias-to-ID mapping and backup reference together in
`workspace-migration.json`. The original configuration, stores, accounts,
and grants remain authoritative. Legacy wildcard scopes are untouched. No
new scope is granted. Startup can validate and read the staged catalog,
while navigation and data access still use the historical implementation.

An interruption before publication leaves an unpublished staging operation.
Run apply again to create a fresh backup. Any earlier backup or partial
copy is retained for inspection. An interruption after publication leaves
a staged catalog. Verify checks the referenced backup and its manifest.
Rollback atomically marks the staged catalog withdrawn and retains the
backup. It never replays copied files over the live home, including if new
data was written between staging and rollback. A recorded rollback can be
run again safely.

The marker uses atomic replacement and file synchronization. On Unix the
parent directory is also synchronized. Windows process-interruption tests
do not establish persistence across a sudden power loss. Confirmed
historical records whose authentication already fails still require
recovery from a known-good backup. The procedure does not rewrite them.

## Administrative hierarchy preview

Start the shell with `kivadb --hierarchy-preview` or enter `hierarchy on`
from an administrator session. A TCP connection must authenticate as an
administrator. The preview starts at `kiva[default]>` and tracks workspace,
project, and collection identities independently for each connection.

```text
workspace create fordima sandbox
switch fordima
project create efoma
use efoma
collection create users products orders
use collection users
use efoma.products
use collection fordima.efoma.orders
hierarchy off
```

`switch` always selects a workspace. An unqualified `use` selects a project.
Changing workspace clears the project and collection. Changing project clears
the collection. Failed navigation keeps the prior valid context. A rename
follows an existing identity across connections. If another connection drops
that identity, subsequent lookups fall back to the closest surviving parent.
Recreating the name does not rebind the selection.

The `workspace`, `project`, and `collection` commands each accept `create`,
`list`, `current`, `info`, `exists`, `rename <old> <new>`, and `drop`.
Project administration is relative to the current workspace. Collection
administration is relative to the current project. Each batch of names passed
to `create` commits as one system record. Dropping a nonempty workspace or
project requires `cascade`. The default workspace cannot be renamed or
dropped.

The preview catalog is a separate system record for metadata experiments.
It allocates no data stores and never supplies a legacy engine lookup.
Hierarchical access is not enabled. Data commands, structured data requests,
and raw binary reads and writes are refused while preview is active.
`hierarchy off` restores the session's historical selection.

When offline migration is staged, the preview shows that catalog read-only.
Legacy clients can still read and write existing stores. Legacy registration
changes, storage path changes, single-store restore into the home, and
incoming replication are blocked while that stage is active. A rollback
withdraws the stage and makes the independent preview metadata visible again.
Activating hierarchical data routing will require the grant adapter,
protocol and maintenance changes before choosing an authoritative catalog.

## Physical layout export

The managed path is relative to an operator-selected directory:

```text
workspaces/<workspace>/projects/<project>/collections/<collection>/store.kiva
```

Only collections contain store files. The export also creates directories for
empty workspaces and projects. Relative paths use catalog names, while the
manifest records the collection IDs. Windows device names and invalid name
segments are refused on all platforms.

Stop the source instance and stage migration with `migrate apply`. Use
`migrate layout-plan` to see the mapping and then run:

```sh
kivadb migrate copy-layout --home /path/to/source --target /path/to/new-export
```

The target parent must already exist and its path must use real directories.
Choose a new target outside the source home and all registered source stores.
The command never writes to an existing destination. It copies the verified
backup snapshot and checks each target file with SHA-256. The result is a
point-in-time snapshot, so it excludes source writes made after staging.
Custom source stores stay registered at their original paths.

The export marker moves from `COPYING` to `COMPLETE` after all copies verify.
A partial target is retained for inspection. Retry into another new target.
Both marker states prevent ordinary startup in that directory. No source file
is renamed or deleted. Rolling back the offline migration withdraws its
catalog and leaves the backup and any exports available.

This export contains collection files and relative layout metadata. Serving
configuration, account migration, active path authority, online allocation,
physical rename and retirement require subsequent integration. The preview
catalog remains independent from these files. A completed export cannot
activate hierarchical routing.


For activation, the planned authority is a single versioned catalog record
in the system engine. The migration and export manifests are inputs to a
verified cutover and recovery procedure. The experimental preview record
stays separate and is never merged into an active catalog. A future change
must define the atomic transition and bind each active collection ID to its
verified managed path before allowing writes.

## Managed storage administration

On an unstaged test home with the standard legacy default store and no
other legacy registrations, start `kivadb --no-listen --home <home>`.
Enter `hierarchy storage` to use the managed storage catalog. The command
is restricted to the local administrator session with no TCP listener.
`hierarchy on` still opens the separate metadata-only preview.

```text
hierarchy storage
workspace create Trading
switch Trading
project create MaplaTrade
use MaplaTrade
collection create agents positions transactions
collection rename agents to workers
collection drop workers
```

Managed workspace and project directories contain no user store. Each
collection is initialized with the home engine options and carries a
marker containing its catalog ID and its store header ID. Starting the
instance validates both identities and prevents two resources from
sharing a primary file. Managed data remains inaccessible through shell
data commands or clients. Layout administration does not import a
staged migration or merge the preview catalog.

The managed catalog and its optional pending snapshot are a single atomically
replaced record in `system/managed-layout.json`. Edits to that record are
serialized in the instance. The record holds the intent before any move.
On restart, recovery recognizes each move endpoint by its immutable marker.
It completes outstanding moves, validates the resulting tree, and publishes
the next catalog. An existing directory is never adopted as a newly managed
resource. Case-only renames are refused for consistent cross-platform behavior.
A conflicting open store prevents moving its directory. External filesystem
edits during administration are unsupported.

A drop moves the top-level affected resource to `retired/<hex-id>`. A
cascade keeps the subtree intact, including its identity markers and stores.
A later creation assigns a new ID and initializes a fresh store. To recover
retired data, retain the directory and its markers and work from a copy with
the original key configuration. Reattachment to a live catalog is a separate
operation and is not supported yet. Interrupted initialization can leave an
unused scratch directory under `system/managed-layout-staging`. Recovery
starts a fresh scratch directory and never deletes an abandoned copy.

Legacy topology changes and legacy migration apply are refused in a managed
home. Legacy full-home backup is also refused because its archive format
does not cover the managed files. Legacy reads and writes to the existing
default store still use the existing engine. Unix directory synchronization
and Windows process-crash recovery have different durability guarantees.
Physical relocation across mounts and unmanaged edits are outside this
administrative mode.

## Hierarchy authorization before routing

Managed metadata grants are keyed by a stable account identity and a typed
instance, workspace, project, or collection scope. Resource names are resolved
only when granting or revoking. Each permission flows down to descendants
through their IDs. A direct grant makes the ancestors visible for navigation,
without granting access to their other descendants. Unknown and hidden
qualified paths return the same denial to non-administrators.

In the local managed layout, an account administrator can assign permissions:

```text
hierarchy storage
hierarchy grant editor read collection Fordima.Efoma.users
hierarchy grant editor write project Fordima.Efoma
hierarchy grant editor manage workspace Fordima
hierarchy revoke editor write project Fordima.Efoma
```

An instance grant takes no path argument. `write` implies `read`. `manage`
permits metadata administration only. Creation and rename require management
of the namespace where a name is registered. Drop requires management of the
target. Viewers remain read-only. Only account administrators can edit policy.
`hierarchy permissions` reports policy decisions on the selected resource.
Managed data access requires the separate opt-in described below.

The policy is one versioned system-engine record, encrypted when the home
uses encryption. Policy updates and managed metadata commands share the
managed catalog lock. Commands refresh the account and policy, so a prior
selection does not retain an old grant. Renames preserve the same IDs.
Retired IDs are inert and never reassigned. Account recreation also receives
a new identity. Older accounts without an explicit identity derive one
from their existing password salt and persist it on the next account save.
An old executable dropping the identity field may invalidate new grants.

An offline migration stays read-only. Its metadata view projects historical
permissions onto exact collection IDs using each alias's recorded original
name. It preserves the result of the historical scope matcher and does not
rewrite legacy grants or activate a new data policy.

| Historical grant | Historical name | Staged collection | Decision |
| --- | --- | --- | --- |
| `read on Efoma.*` | `Efoma.users` | `default.Efoma.users` | Read |
| `read on Efoma.*` | `Efoma.products` | `default.Efoma.products` | Read |
| `read on Efoma.*` | `Fomapay.users` | `default.Fomapay.users` | Denied |
| `read on Efoma.*` | `default` | `default.legacy.default` | Denied |
| `write on lab` | `lab` | `default.legacy.lab` | Read and write decision for an editor |

Unknown policy versions and malformed grants stop startup. Historical
wildcards are never converted into broad workspace or project management
rights. A future active routing cutover must persist or reevaluate these exact
collection decisions and check them before opening engines.

## Local managed data routing

Start the local shell with `kivadb --home <home> --no-listen` and enter
`hierarchy data` in a standard, unstaged home. Select a workspace, a project
and a collection, or resolve an authorized fully qualified name with
`use collection Fordima.app.users`. `hierarchy off` leaves this session mode.
`hierarchy storage` continues to provide metadata administration.

Each admitted command holds the shared side of the layout barrier from
authorization through execution and synchronous publication. Topology edits,
account mutations and grant changes take the exclusive side. A revoke waits
for previously admitted commands, and subsequent commands check the current
policy. A transaction holds an engine reference between commands. Commit
takes a new shared guard, refreshes the account, and discards pending writes
if access has been revoked.

The lock order is layout barrier, then catalog or policy cache, followed by
registry admission and command execution. The registry mutex is released
before executing a command. A cold open or eviction holds the registry
mutex while verifying storage or closing an idle engine. Store internals
still coordinate access through the existing engine locks. Compaction takes
the exclusive layout barrier for its full duration. Rename and retirement
wait for running operations and fail if any managed transaction still pins
a store. Account identity is checked under the same layout barrier on
metadata and data paths.

The managed catalog is an immutable shared snapshot and ID lookups use
binary search. Policy deserialization builds a grant index by account
and immutable subject ID. The system record version identifies the cached
policy. Data permission checks consult exact grants along the actual parent
chain. Metadata visibility can also inspect descendant grants. Reads and
queries require Read. Mutations require Write. Index, vector, schema and
journal definitions, integrity checks and compaction require Write and
Manage. Named journal cursors require Write.

### Managed engine registry

The registry keeps up to `storage.managed_open_engines` engines, with a
default of 32 and a range from 1 to 1024. Config changes apply on restart.
The least recently used idle engine is eligible for eviction. A running
operation or transaction retains a reference, and saturation reports a
retryable command error. On a cold open the engine verifies the collection
marker and physical file identity. Topology changes invalidate the registry
before changing any paths. `hierarchy engines` reports aggregate registry
counts to instance administrators.

### Compaction and recovery

`compact` rewrites the selected managed collection through the same
engine used by the legacy path. The collection ID and path stay stable.
The compacted file gets a new physical generation ID. The managed wrapper
requires the replacement to be synced and records the old and replacement
IDs in a per-collection intent before the engine swaps the live file.
Compaction refuses unreadable live records and an active transaction on the
target collection.

Open and topology validation check the live file header against the marker
and intent. If the old generation is present, recovery keeps it. If the
prepared generation is live, recovery publishes its verified identity,
then removes the intent and any temporary file. An unexpected generation
fails validation. Recovery never installs a temporary file. Unix syncs the
directory before publishing and after cleanup. Windows requests write-through replacement for the store and its markers. The recovery tests simulate
process interruption. Sudden power loss on Windows remains unverified.

The allowlist covers key and document commands, native types, queries,
index/schema/vector operations, transactions, change journal, compaction
and integrity checks. Managed engines sync writes before acknowledgment.
TTL uses on-access expiry. Scripts, triggers, subscriptions, staged
migration, replication and backup/restore remain unavailable in this mode.

Phase 7A adds a separate native TCP listener mode for initialized managed
homes. The negotiated connection keeps its own authenticated hierarchy
context and uses the same layout barrier, grants and registry. Native frame
limits, supported operations and disabled adapters are covered in the
[native hierarchical protocol](NATIVE_HIERARCHICAL_PROTOCOL.md). See
[readiness and measurements](../testing/workspace-hierarchy-readiness.md).

### Native admission and layout ordering

Phase 7A.2 adds independent FIFO admission lanes for native requests.
The lock order is request lane, layout turnstile, layout barrier, caches,
registry and engine. Exclusive operations hold the turnstile while waiting
for previously admitted layout operations to finish. The turnstile is
released once the layout guard is acquired. A confirmed revoke has drained
all operations previously admitted under the shared guard. Socket replies
follow release of the lane and layout guards.

Compaction still uses the exclusive instance barrier. Per-store compaction
would need to coordinate the generation journal, registry eviction, layout
moves and transaction pins across the swap. That work remains separate.

### Managed REST and RESP adapters

The optional adapters reuse authenticated native sessions and the managed
layout barrier, grant checks, engine leases, and transaction path. REST
resolves the complete resource path per request. RESP selects a stable
collection identity per connection using `KIVA.USE`. Both bind dedicated
listeners behind explicit flags. The [adapter guide](HIERARCHICAL_REST_RESP.md)
covers activation, operations, limits, error semantics, and TLS.

### Managed REST authentication

An optional REST lease binds a random token fingerprint to the immutable
account ID and a fingerprint of its stored password verifier. Each HTTP
request reconstructs its own account context. The shared hierarchy barrier
covers authorization through write commit. Logout and administrative token
revocation use the exclusive side of that barrier. HTTP Basic continues to
use the managed password admission lane. See the [REST session guide](HIERARCHICAL_REST_RESP.md#rest-authentication-and-sessions).
