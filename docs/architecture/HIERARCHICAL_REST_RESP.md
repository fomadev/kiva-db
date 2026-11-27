# Hierarchical REST and RESP

Phase 7B is experimental. Prepare a home with authenticated accounts and
`hierarchy data` before starting the managed listeners. Each adapter is an
independent opt-in and uses the same managed catalog, grants, engine registry
and transaction implementation as native TCP.

```sh
kivadb serve --hierarchical-tcp --hierarchical-http --hierarchical-resp \
  --bind 127.0.0.1 --port 2911 --http-port 8080 --resp-port 6379
```

The same flags work with `kivadb start`. A managed instance uses dedicated
listeners and turns off the historical REST and RESP routes. Historical
instances retain their existing configuration and behavior. Bind to loopback
for local plaintext development. A non-loopback managed listener requires
server TLS and `tls.required=true`. Clients should verify the certificate.
Use a verified local TLS endpoint with `https://` or `redis-cli --tls --cacert`.

## REST

Clients can send HTTP Basic credentials on each request or exchange them
for an opaque Bearer token. `GET /health` is public and reports the listener
mode. Each request carries its full resource path and gets a fresh execution
context, including on persistent connections and when reusing a token.

| Route | Methods |
| --- | --- |
| `/v2/workspaces`, `/v2/workspaces/{w}/projects`, `/v2/workspaces/{w}/projects/{p}/collections` | GET list, POST create with `{"name":"new"}` |
| An item under one of the routes above | GET info, PUT rename with `{"name":"new"}`, DELETE retire |
| `…/collections/{c}/keys/{key}` | GET, PUT typed value, DELETE |
| `…/collections/{c}/api` | POST structured `get`, `put`, `del`, `add`, `scan`, `query`, `explain`, `aggregate`, `similar`, `changes` and `whoami` |
| `…/collections/{c}/batch` | POST up to 64 ordered `put`/`del` operations in one optimistic transaction |
| `…/collections/{c}/maintenance` | POST `{"command":"indexes"}` or another managed index, schema, changes, compact or check operation |

Retiring a parent with children requires `{"cascade":true}`. Key routes
default to extended JSON. Structured requests can opt into extended JSON
with `"types":"extended"`. Query, scan and similarity requests default
to 100 results and permit up to 1,000. A batch rolls back on any staging
error and keeps no server transaction between requests.

```sh
base=http://127.0.0.1:8080/v2/workspaces/Fordima/projects/app/collections/users
curl -u editor -H 'Content-Type: application/json' -X PUT "$base/keys/greeting" \
  -d '{"type":"string","value":"hello","ttl":60}'
curl -u editor "$base/keys/greeting"
curl -u editor -H 'Content-Type: application/json' -X POST "$base/batch" \
  -d '{"operations":[{"op":"put","key":"active","type":"bool","value":true},{"op":"del","key":"greeting"}]}'
```

Errors have `error` and `message` fields. 400 means invalid input, 401 means
authentication is required or failed, 403 means forbidden, and 404 covers
both missing and hidden resources. 409 covers conflicts and 422 covers
schema or adapter result limits. 413 and 431 reject oversized requests and
headers. 503 `server_busy` is a pre-execution refusal. On 500
`outcome_unknown` or a broken connection after sending a write, verify
the state before retrying a potentially non-idempotent operation.

## REST authentication and sessions

| Method and route | Authorization | Result |
| --- | --- | --- |
| POST `/v2/auth/login` | Basic | 201 with `access_token`, `token_type` and session metadata |
| GET `/v2/auth/session` | Bearer | Name of the authenticated account and the creation and expiry times in Unix seconds |
| POST `/v2/auth/logout` | Bearer | Revokes this token and returns 200 |
| POST `/v2/auth/revoke` | Administrator via Basic or Bearer | Revokes an account's existing sessions using `{"account":"editor"}` and returns the count |
| GET `/v2/auth/metrics` | Administrator via Basic or Bearer | Registry counters and opt-in timing aggregates |

```sh
server=https://localhost:8080
cert=/path/to/kivadb-home/tls/cert.pem
token=$(curl --silent --show-error --fail --cacert "$cert" -u editor \
  -X POST "$server/v2/auth/login" | jq -er .access_token)
printf 'Authorization: Bearer %s\n' "$token" |
  curl --fail --cacert "$cert" -H @- "$server/v2/auth/session"
printf 'Authorization: Bearer %s\n' "$token" |
  curl --fail --cacert "$cert" -H @- -X POST "$server/v2/auth/logout"
unset token
```

Tokens are 256 random bits from the system generator and are sent in the
Authorization header. The process retains SHA-256 fingerprints, immutable
account IDs, verifier generations and expiry metadata. A token carries no
stored grant or collection selection. The adapter resolves the current
account and hierarchy access on each request. An invalid, expired, logged
out or stale token returns 401. Restart invalidates every token. Clients
can log in again after expiry if their credentials still work.

| Startup setting | Default | Accepted range |
| --- | ---: | ---: |
| `http.session_capacity` | 4,096 | 1–65,536 |
| `http.sessions_per_account` | 256 | 1–4,096 |
| `http.session_ttl_seconds` | 900 | 1–86,400 |

The lifetime is absolute. Saturation returns 429 `session_capacity`. Existing
sessions keep their lifetime. Expired entries are reclaimed on lookup,
issuance and administrative metrics requests. An account's next successful
login also removes its stale verifier and identity generations. The registry
is process-local and bounded, including between cleanup passes. It stores
only authorization leases. Basic authentication still verifies each request.

Logout and administrative revocation take the exclusive managed layout
barrier. Each data operation accesses its lease and live account under the
shared barrier and holds that barrier through the write or batch commit.
An admitted request can finish before the revocation obtains the barrier.
Once revocation succeeds, queued or subsequent operations using that token
fail. Grant changes use the same barrier. Password changes invalidate the
verifier generation, and deleting and recreating a name changes the account
ID. Administrative revoke clears existing sessions and leaves the account
eligible to log in again. Audit entries and timing aggregates carry no
Authorization header, password or token. Send tokens over verified TLS
outside loopback.

HTTP Basic and login requests share the bounded password lane with native
authentication. Under saturation, clients may receive 503 before password
verification or dispatch. Bearer lookup runs in memory and uses the same
connection cap and Read, Data and Control lanes. Idle and slow connections
still have a whole-frame deadline. The native TCP path retains its existing
FIFO admission behavior. The managed REST and RESP adapters use timed waits.

## RESP

Run the following in one authenticated `redis-cli` connection. The initial
context is empty. Reauthentication clears it. Select the collection again
after reconnecting. Renames preserve selected identities. Retiring and
recreating a name requires an explicit new selection.

```text
AUTH editor <password>
KIVA.USE Fordima.app.users
SET greeting hello EX 60
GET greeting
MULTI
SET active yes
DEL greeting
EXEC
```

| Coverage | Commands and semantics |
| --- | --- |
| Supported | RESP2 and RESP3 handshake, AUTH, HELLO, PING, ECHO, QUIT, CLIENT SETINFO, GET, SET, DEL, UNLINK, EXISTS, TYPE, MGET, MSET, INCR, DECR, INCRBY, DECRBY, EXPIRE, PEXPIRE, EXPIREAT, PEXPIREAT, TTL, PTTL, EXPIRETIME, PEXPIRETIME, PERSIST |
| Partial | SCAN and KEYS are collection-scoped, page/result-bounded and unavailable inside MULTI. Keys must be valid KivaDB UTF-8 keys. Values follow KivaDB's native storage and scalar conversions. Multi-key reads outside a transaction can observe concurrent writes. |
| Transactions | MULTI/EXEC/DISCARD queue scalar operations on one collection. EXEC requires Write, rechecks the selected identity and grant, then uses an optimistic transaction. Redis-style command errors can appear in the EXEC array. A conflict or transaction-level failure aborts the commit. |
| Extensions | KIVA.USE selects a collection. KIVA.API accepts the collection-scoped structured data API, except explicit transaction operations and whoami. |
| Unavailable | SELECT, WATCH/UNWATCH, inline framing, global scans and FLUSH, scripting, Redis data-structure writes, push and pub/sub, and replication. |

Pipelines run in arrival order and replies are written in that order.
`NOPERM` covers forbidden or obsolete resources, `BUSYERR` denotes admission
refusal, and `TRYAGAIN` covers optimistic conflicts. On `ERR_OUTCOME_UNKNOWN`
or an interrupted connection after submitting a write, verify the state
before retrying. Disconnecting before EXEC discards the connection's queue.

## Security and resource limits

Grants are evaluated against stable account and collection identities. Read
permits lookup, Write permits mutation, and Manage or the applicable admin
role gates lifecycle and maintenance. Viewer restrictions still apply. Each
data operation uses the shared layout barrier and selected engine lease.
In-flight work may finish before a revocation acquires that barrier.
Subsequent work and transaction commits reauthorize.

The listeners share the native connection cap of 128 and the native Read,
Data, Authentication and Control admission lanes. Adapters wait at most two
seconds for admission. The deadline applies to queued work. Engine operations
already admitted can finish after the client disconnects. The whole-frame
read and write deadlines are 30 seconds. Neither adapter keeps an
application-level pipeline or output queue.

HTTP rejects duplicate headers, transfer encoding, query strings and
ambiguous path components. It decodes paths once, limits the header to
16 KiB, limits bodies to 2 MiB, and closes a persistent connection after
128 requests. It uses no cookies or CORS headers. RESP accepts arrays of up
to 128 bulk strings with a 2 MiB payload budget and requires CRLF. A
transaction retains at most 64 frames and 2 MiB of their payloads.
Adapter values and staged writes have an additional 2 MiB budget. An
individual response is capped at 4 MiB. Vector search bounds decoded
candidate bytes and reads the selected managed engine directly. Other
engine working sets still depend on engine configuration. Reserve
plaintext for local access and keep credentials out of URLs and command
histories.

## Routing and permission audit

The version prefix follows the historical listener's short resource paths.
An explicit `/v2/workspaces` tree gives these routes a distinct namespace.
`http.rs` and `resp.rs` retain historical behavior. The opt-in implementations
start in `http_managed.rs` and `resp_managed.rs` and reuse `tls.rs`,
`managed_transport.rs` and the native lanes in `native_admission.rs`.
`Session::with_managed_key` and `Session::with_managed_engine` enforce
`hierarchy_auth.rs` and enter `managed_layout.rs` before using `engine.rs`.
The stable account and object identities come from the account policy and
`catalog.rs`.

| Historical capability | Managed availability | Required access |
| --- | --- | --- |
| HTTP health, RESP PING | Public liveness only | None |
| HTTP key/document operations, RESP scalar reads and scans | Scoped to a selected collection | Read |
| HTTP structured mutations and batch, RESP scalar mutations and EXEC | Scoped to one collection | Write |
| HTTP queries, explain, aggregates, similarity and journal reads | Structured API with bounded responses and journal polling | Read |
| HTTP consumer checkpoint changes | Structured API | Write |
| HTTP lifecycle operations | Catalog-backed list, info, create, rename and retire | Visibility to list or view, Manage on the applicable resource for mutations, admin for instance-level operations |
| HTTP index/schema definitions, journal configuration, check and compaction | Filtered managed maintenance operations | Existing Read or Manage requirement per operation |
| HTTP OpenAPI, metrics, WebSocket, and legacy shell/administration routes | Historical listener only | Historical policy |
| RESP global commands, numeric databases, hashes/lists/sets, Lua, pub/sub and WATCH | Historical listener only where implemented | Historical policy |
