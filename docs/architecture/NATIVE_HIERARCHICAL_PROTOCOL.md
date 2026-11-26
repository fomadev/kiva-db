# Native hierarchical protocol

## Startup and negotiation

Initialize the managed catalog and provision a password-protected
administrator from an offline shell with the listener disabled. Enable
`security.auth_required`. Start the configured home with
`kivadb serve --hierarchical-tcp --home <home>`. `start` accepts the same
flag. This mode requires an initialized managed catalog and a standard
unstaged home. Non-loopback listeners require enabled TLS and
`tls.required=true`. REST, RESP, replication and failover are unavailable.

The frame is unchanged: a big-endian u32 length counting the u16 code and
payload. A length within the hierarchical limit starts with a zero byte.
The server accepts framed commands on this listener. The legacy line
transport returns a negotiation error.

| Request | Payload | Reply |
| --- | --- | --- |
| 1 HELLO | Empty | Banner with `auth=required` and `capability=hierarchy/1` |
| 9 NEGOTIATE | UTF-8 `hierarchy/1` | Status 0 with the same capability |
| 2 AUTH | UTF-8 username, NUL, UTF-8 password | Status 0 or a login error |
| 3 EXEC | UTF-8 shell command | Line-tagged output and existing status codes |
| 4 PING | Empty | PONG |
| 5 GET | UTF-8 key | Existing type tag and value, or not found |
| 6 SET | Existing type, TTL, key length, key and value fields | Existing status |
| 7 DEL | UTF-8 key | Existing status |
| 8 API | UTF-8 JSON request | JSON response, or a status 1 error |

HELLO and PING are available before negotiation. NEGOTIATE is accepted
once per connection, before AUTH. Older clients receive a capability
error from an unnegotiated hierarchical connection. Historical servers
keep their existing behavior, and return an error for NEGOTIATE.

The first successful login starts with a fresh hierarchy context and no
selected collection. The account must supply a nonempty password even over
loopback. Authenticate with the account name, then use `switch <workspace>`,
`use <project>`, `use collection <collection>`, `use <project>.<collection>`
or `use collection <workspace>.<project>.<collection>`. The session pins ID
selections independently of other connections. Renaming preserves an ID.
Recreating a retired name allocates a different ID. Mode switching is
unavailable on an admitted hierarchy connection.

## Route audit

| Entry point | Hierarchy behavior |
| --- | --- |
| Native socket and TLS | `src/server.rs` accepts connections through `src/tls.rs` and admits managed requests through the negotiated session |
| Legacy line TCP | Refused before dispatch in hierarchical mode |
| Framed EXEC | `Session::execute` uses the managed command allowlist and catalog |
| Framed GET, SET and DEL | Resolve through `with_managed_engine` before calling the existing session KV methods |
| Framed API | An explicit allowlist dispatches to the existing JSON data handlers under the same managed admission |
| REST and RESP | Their listener entry points reject a hierarchical instance |
| Replication and failover frames | Refused at the negotiated listener |
| Embedded Rust Engine and Session | Low-level local facilities. Network callers receive the authenticated `Client` interface |

Framed EXEC supports the existing managed collection operations, including
index, schema, vector, journal and compaction commands. The structured
allowlist is `whoami`, `get`, `put`, `add`, `del`, `scan`, `query`,
`explain`, `aggregate`, `similar`, `changes`, `begin`, `commit` and
`rollback`. Use the existing type tags and extended JSON rules for native
values. A journal read uses `wait=0`. A named consumer requires Write.
Push subscriptions and cross-session semantic caches are unavailable.
Journal polling rechecks grants on every request. Path-bearing diagnostics
and incompatible administrative operations are unavailable. Administrators
can create and drop accounts through the restricted EXEC forms and can
edit hierarchy grants. Provision or change passwords for existing
accounts from an offline local shell.

## Admission and transactions

The selected IDs and authenticated account are checked under the existing
shared layout barrier before registry lookup. Read, Write and Manage have
the same meanings as in the local hierarchy. Revocation and account
mutations take the exclusive side. A successful revocation has drained
earlier admitted operations. Replies from already admitted operations can
still be buffered in the transport. Push delivery is unavailable in this
phase.

A transaction belongs to one connection and one collection. Commit
reauthorizes against current account identity, role and grants. A disconnect
or idle timeout rolls back an unpublished transaction and releases its
registry pin. If a transport failure hides the commit response, reconnect
and verify application state. The client does not replay a commit or write.
Data handlers release the layout and engine guards before a reply is written.

Hierarchical mode admits at most 128 sockets per process and honors a lower
`server.max_clients`. Requests are limited to 2 MiB and responses to 4 MiB
including the code. AUTH payloads are limited to 4 KiB and EXEC to 64 KiB.
Each frame has a 30-second read or write deadline. Malformed lengths or
truncated frames close the connection. Malformed UTF-8, JSON and typed
values yield a command error on a valid frame. Replies above the limit
yield an error. Large query intermediates remain governed by the existing
engine and query settings. The server uses its configured execution slots
and retains no per-client notification queue in hierarchical mode.
