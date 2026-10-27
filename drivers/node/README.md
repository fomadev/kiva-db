# @fomadev/kivadb — Node.js driver

The official Node.js driver for [KivaDB](../../README.md). It speaks KivaDB's binary protocol on port 2911, has no dependencies, and ships TypeScript types. It needs Node.js 18 or later and a running KivaDB (`kivadb start`).

## Install

```bash
npm install @fomadev/kivadb
```

The driver has its own version numbers, separate from the server's:

| Driver | Tested with | Needs a newer server |
| :--- | :--- | :--- |
| 1.2.0 | KivaDB 2.0.0 | `date`, `datetime` (a `Date`) and `time`: 2.0.0 or later |
| 1.1.0 | KivaDB 1.7.0 | `add`, and `in` / `like` / `ilike` in `query`: 1.7.0 or later |
| 1.0.0 | KivaDB 1.6.0 | `semanticCached`: 1.6.0 or later |

The other methods work with any server from 1.6.0.

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
| `get(key)` | The value, or `undefined`. Bytes come back as a `Buffer`, a `datetime` as a `Date`, a `date` (`"2026-10-27"`) and a `time` (`"14:30:00"`) as text |
| `set(key, value, {ttl, mode, type})` | Creates or replaces. The KivaDB type follows the JavaScript value (`string`, `number`, `boolean`, `null`, array, object, `Buffer`, `Date` for a `datetime`). For a `date` or a `time`, give the text and the type: `set("born", "2004-05-09", { type: "date" })`. `mode`: `upsert` (default), `create`, `update`. `ttl`: seconds; `0` removes an expiry; omitted keeps it |
| `add(prefix, value, {ttl, type})` | Stores the value under a key KivaDB names: `add("place:", {name: "Goma"})` resolves to `{key: "place:6adf9c3c...", id: "6adf9c3c..."}`. The identifier has MongoDB's ObjectId format and sorts by creation time |
| `query(pattern, where, {limit})` | The documents that satisfy a condition, in key order: `query("place:*", {type: "VILLE", parent: null})`, `query("place:*", {field: "name", cmp: "ilike", value: "kin%"})`, `query("place:*", {parent: ["place:1", "place:2"]})` (a list means `in`), `{and: [...]}`, `{or: [...]}`, `{not: {...}}`. Resolves to `{key, value}`; `limit` defaults to 100 |
| `del(key)` | Resolves to whether the key existed |
| `scan(pattern, limit)` | Keys with type, size, modification time and TTL |
| `similar(target, vector, {limit, metric, ef, exact})` | Vector search: `similar("doc:*.embedding", [0.1, ...], {limit: 5})` returns `{key, score, value}` best first. Metrics: `cosine` (default), `dot`, `l2`. With an HNSW index, `ef` trades speed for accuracy and `exact: true` compares every vector |
| `exec(command)` | Any command of the KivaDB shell language; resolves to the output lines, rejects with `KivaError` on an error |
| `semanticLookup(cache, {text, vector, threshold})` / `semanticStore(cache, answer, {text, vector, ttl, depends, since})` | Semantic cache: reuse the answer of a question already asked, or of a near-identical one (DOCUMENTATION 12.8) |
| `semanticCached(cache, {text, vector, threshold, ttl, depends}, compute)` | The cached answer, or `compute()`'s, stored unless the data it depends on changed while it was computed |
| `api({op, ...})` | Structured request (`whoami`, `workspaces`, `scan`, `get`, `put`, `add`, `del`, `query`, `explain`, `aggregate`, `similar`, `semantic`, `stats`, `users`) |
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

When KivaDB's message starts with a word in capitals and a colon, `err.code` holds that word, so code can test it without comparing the whole text. A write that a unique index refuses (KivaDB 1.7.0 or later) has the code `UNIQUE`:

```js
try {
  await db.add("place:", { type: "VILLE", parent: null, name: "Goma" });
} catch (err) {
  if (err.code === "UNIQUE") return reply(409, "This place already exists.");
  throw err;
}
```

Other codes are `READONLY` (the server is a replica) and `ERR_TRANSACTION_CONFLICT`.

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
