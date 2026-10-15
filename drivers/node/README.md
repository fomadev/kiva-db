# kivadb — Node.js driver

The official Node.js driver for [KivaDB](../../README.md). It speaks KivaDB's binary protocol on port 2911, has no dependencies, and ships TypeScript types.

```bash
npm install kivadb
```

## Quick start

```js
const { connect } = require("kivadb");

const db = await connect({ host: "localhost", port: 2911, user: "kiva", password: "" });

await db.set("user:1", { name: "Alice", role: "admin" }, { ttl: 3600 });
console.log(await db.get("user:1"));            // { name: 'Alice', role: 'admin' }

console.log(await db.exec('count where user:*.role = "admin"'));   // [ 'Count: 1' ]

db.close();
```

A remote host always needs a password: the driver refuses to connect without one, as KivaDB itself does.

## API

| Method | Does |
| :--- | :--- |
| `connect({host, port, user, password, timeout})` | Opens a connection and logs in |
| `get(key)` | The value, or `undefined`. Bytes come back as a `Buffer` |
| `set(key, value, {ttl, mode, type})` | Creates or replaces. The KivaDB type follows the JavaScript value (`string`, `number`, `boolean`, `null`, array, object, `Buffer`). `mode`: `upsert` (default), `create`, `update`. `ttl`: seconds; `0` removes an expiry; omitted keeps it |
| `del(key)` | Resolves to whether the key existed |
| `scan(pattern, limit)` | Keys with type, size, modification time and TTL |
| `similar(target, vector, {limit, metric})` | Vector search: `similar("doc:*.embedding", [0.1, ...], {limit: 5})` returns `{key, score, value}` best first. Metrics: `cosine` (default), `dot`, `l2` |
| `exec(command)` | Any command of the KivaDB shell language; resolves to the output lines, rejects with `KivaError` on an error |
| `api({op, ...})` | Structured request (`whoami`, `workspaces`, `scan`, `get`, `put`, `del`, `similar`, `stats`, `users`) |
| `subscribe(pattern)` | Then `db.on("change", e => ...)` receives `{event, key, type}` for every set, update and delete |
| `stats()`, `ping()`, `close()` | |

Requests can be sent concurrently on one connection; answers come back in order.

## Tests

The tests start a real KivaDB. Build it first (`cargo build` in the repository), or point `KIVADB_BIN` at an executable:

```bash
npm test
```
