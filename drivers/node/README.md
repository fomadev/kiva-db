# @fomadev/kivadb — Node.js driver

The official Node.js driver for [KivaDB](../../README.md). It speaks KivaDB's binary protocol on port 2911, has no dependencies, and ships TypeScript types. It needs Node.js 18 or later and a running KivaDB (`kivadb start`).

## Install

```bash
npm install @fomadev/kivadb
```

The driver has its own version numbers, separate from the server's: driver 1.0.0 is tested with KivaDB 1.6.0, and `semanticCached` needs a server of 1.6.0 or later.

To try a version not yet published, install it from a copy of the repository:

```bash
npm install path/to/kiva-db/drivers/node
```

## Quick start

```js
const { connect } = require("@fomadev/kivadb");

const db = await connect({ host: "localhost", port: 2911, user: "kiva", password: "" });

await db.set("user:1", { name: "Alice", role: "admin" }, { ttl: 3600 });
console.log(await db.get("user:1"));            // { name: 'Alice', role: 'admin' }

console.log(await db.exec('count where user:*.role = "admin"'));   // [ 'Count: 1' ]

db.close();
```

A remote host always needs a password: the driver refuses to connect without one, as KivaDB itself does.

Encrypt the connection with TLS, checking the server's certificate by the fingerprint that `kivadb tls` shows on the server:

```js
const db = await connect({
  host: "31.25.236.110",
  password: process.env.KIVADB_PASSWORD,
  tls: { fingerprint: "77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6" },
});
```

With a certificate from an authority (your own domain), `tls: true` is enough.

## API

| Method | Does |
| :--- | :--- |
| `connect({host, port, user, password, timeout, tls})` | Opens a connection and logs in; `tls`: `true` or `{ fingerprint }` |
| `get(key)` | The value, or `undefined`. Bytes come back as a `Buffer` |
| `set(key, value, {ttl, mode, type})` | Creates or replaces. The KivaDB type follows the JavaScript value (`string`, `number`, `boolean`, `null`, array, object, `Buffer`). `mode`: `upsert` (default), `create`, `update`. `ttl`: seconds; `0` removes an expiry; omitted keeps it |
| `del(key)` | Resolves to whether the key existed |
| `scan(pattern, limit)` | Keys with type, size, modification time and TTL |
| `similar(target, vector, {limit, metric, ef, exact})` | Vector search: `similar("doc:*.embedding", [0.1, ...], {limit: 5})` returns `{key, score, value}` best first. Metrics: `cosine` (default), `dot`, `l2`. With an HNSW index, `ef` trades speed for accuracy and `exact: true` compares every vector |
| `exec(command)` | Any command of the KivaDB shell language; resolves to the output lines, rejects with `KivaError` on an error |
| `semanticLookup(cache, {text, vector, threshold})` / `semanticStore(cache, answer, {text, vector, ttl, depends, since})` | Semantic cache: reuse the answer of a question already asked, or of a near-identical one (DOCUMENTATION 12.8) |
| `semanticCached(cache, {text, vector, threshold, ttl, depends}, compute)` | The cached answer, or `compute()`'s, stored unless the data it depends on changed while it was computed |
| `api({op, ...})` | Structured request (`whoami`, `workspaces`, `scan`, `get`, `put`, `del`, `similar`, `stats`, `users`) |
| `subscribe(pattern)` | Then `db.on("change", e => ...)` receives `{event, key, type}` for every set, update and delete |
| `stats()`, `ping()`, `close()` | |

Requests can be sent concurrently on one connection; answers come back in order.

## Errors

Every method returns a promise. A command that KivaDB refuses (wrong type, missing permission, transaction conflict...) rejects with a `KivaError` whose `message` is KivaDB's own message:

```js
try {
  await db.set("user:1", 42, { mode: "create" });
} catch (err) {
  console.log(err.message);   // Key 'user:1' already exists...
}
```

## TypeScript

```ts
import { connect } from "@fomadev/kivadb";

const db = await connect({ host: "localhost", port: 2911, user: "kiva", password: "" });
```

## More

The protocol, the structured requests and every command are described in the [KivaDB documentation](../../DOCUMENTATION.md), in particular [Chapter 20](../../DOCUMENTATION.md#chapter-20--the-tcp-server) and [Chapter 24](../../DOCUMENTATION.md#chapter-24--connecting-from-your-programs).

## Tests

The tests start a real KivaDB. Build it first (`cargo build` in the repository), or point `KIVADB_BIN` at an executable:

```bash
npm test
```
