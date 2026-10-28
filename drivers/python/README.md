# kivadb — Python driver

The official Python driver for [KivaDB](../../README.md). It speaks KivaDB's binary protocol on port 2911, needs only the standard library, and is typed (`py.typed`). It needs Python 3.10 or later and a running KivaDB (`kivadb start`).

## Install

The driver is not on PyPI yet. Install it from a copy of the repository:

```bash
pip install path/to/kiva-db/drivers/python
```

## Versions

The driver has its own version numbers, separate from the server's:

| Driver | Tested with | Needs a newer server |
| :--- | :--- | :--- |
| 1.0.0 | KivaDB 2.0.0 | `date`, `time` and `datetime` values, `order_by` and `after` in `query`, `explain(analyze=True)` and `changes`: 2.0.0 or later |

The other methods work with any server from 1.7.0. A minor version adds methods, a patch fixes them; a major version changes how existing ones behave, and says how in the changelog of the repository.

## Quick start

```python
import datetime as dt
import kivadb

with kivadb.connect(host="localhost", port=2911, user="kiva", password="") as db:
    db.set("user:1", {"name": "Ana", "role": "dev"}, ttl=3600)
    print(db.get("user:1"))                     # {'name': 'Ana', 'role': 'dev'}

    db.set("order:1:at", dt.datetime.now(dt.timezone.utc))
    print(db.get("order:1:at"))                 # datetime.datetime(..., tzinfo=datetime.timezone.utc)

    rows = db.query("user:*", {"role": "dev"}, order_by="name", limit=20)
    for key, doc in rows:
        print(key, doc["name"])

    with db.transaction():                      # both, or neither
        db.set("acct:a", 10)
        db.set("acct:b", 20)
```

A remote host always needs a password: the driver refuses to connect without one, as KivaDB itself does.

Encrypt the connection with TLS, checking the server's own certificate by the fingerprint that `kivadb tls` shows on the server:

```python
db = kivadb.connect(
    host="31.25.236.110",
    password=os.environ["KIVADB_PASSWORD"],
    tls={"fingerprint": "77:45:F5:56:D2:33:65:C3:02:1A:0C:F7:92:C5:D9:19:BC:44:B7:7D:6F:AF:9A:35:5F:BC:BE:43:CA:12:1A:E6"},
)
```

With a certificate from an authority (your own domain), `tls=True` is enough, or give an `ssl.SSLContext` of your own.

## Values

| Python | KivaDB | Back in Python |
| :--- | :--- | :--- |
| `str` | `string` | `str` |
| `int`, `float` | `number` | `int` or `float` |
| `bool` | `boolean` | `bool` |
| `None` | `null` | `None` |
| `list`, `tuple` | `array` | `list` |
| `dict` | `object` | `dict` |
| `bytes`, `bytearray` | `bytes` | `bytes` |
| `datetime.datetime` with a zone | `datetime` | `datetime` in UTC |
| `datetime.date` | `date` | `date` |
| `datetime.time` without a zone | `time` | `time` |

* **A naive `datetime`** (without `tzinfo`) is refused with a `ValueError`: it would mean another instant on every machine. Give it a zone: `value.replace(tzinfo=datetime.timezone.utc)`, `zoneinfo.ZoneInfo("Africa/Kinshasa")`, or `value.astimezone()` for the machine's own.
* **KivaDB keeps datetimes to the millisecond:** microseconds are cut. A datetime comes back in UTC: the same instant, compared equal to the one written.
* **A `time` with a zone** is refused: a KivaDB time is a time of day, without one.
* **Inside a document** (a `dict` or a `list`), datetimes, dates and times are written as ISO 8601 text (`"2026-10-27T13:30:00.250Z"`), the form KivaDB compares as moments. They come back as that text: the document does not say which texts were moments. Bytes cannot go inside a document.
* `type=` forces a type: `db.set("born", "2004-05-09", type="date")`.

## API

| Method | Does |
| :--- | :--- |
| `connect(host, port, user, password, *, workspace, timeout, tls, reconnect)` | Opens a connection and logs in; `workspace` to work in one; `timeout` in seconds (8) |
| `get(key, default=None)` | The value of a key, or `default` |
| `set(key, value, *, ttl, mode, type)` | Creates or replaces a key; `mode`: `"upsert"`, `"create"` (fails if it exists), `"update"` (fails if it does not) |
| `add(prefix, value, *, ttl, type)` | Stores under a key KivaDB names; returns `(key, id)` |
| `delete(key)` | Deletes a key; returns whether it existed |
| `scan(pattern="*", limit=500)` | Keys with their type, size and TTL |
| `query(pattern, where, *, limit, order_by, after, reverse)` | `(key, document)` pairs; `where` as in the structured requests (`{"role": "dev"}`, `{"field": "age", "cmp": ">", "value": 18}`, `{"and": [...]}`) |
| `explain(pattern, where, *, analyze, ...)` | The plan; with `analyze=True` the query runs and its counts are under `"analyze"` |
| `aggregate(pattern, compute, *, where, group_by)` | `count`, `sum`, `avg`, `min`, `max` |
| `similar(target, vector, *, limit, metric, ef, exact)` | Vector search on `"<pattern>.<field>"` |
| `changes(cursor, *, consumer, limit, pattern, wait)` | The change journal: changes after a cursor or a consumer's position; `wait` seconds for one (at most 30) |
| `transaction()` | `with db.transaction():` commits at the end of the block, rolls back if it raises |
| `subscribe(pattern)`, `next_change(timeout)` | Notifications of writes to keys matching a pattern |
| `exec(command)` | Any command of the shell language; returns its output lines |
| `api(request)` | Any structured request (`{"op": ...}`) |
| `ping()`, `stats()`, `close()` | |

## Errors

| Exception | When |
| :--- | :--- |
| `KivaError` | KivaDB refused the request; `code` is the word before the colon of its message: `UNIQUE`, `SCHEMA`, `CURSOR_EXPIRED`... |
| `AuthenticationError` | Wrong user or password |
| `KivaConnectionError` (also a `ConnectionError`) | The server cannot be reached, or the connection was lost |
| `KivaTimeoutError` (also a `TimeoutError`) | No answer within `timeout` |
| `ValueError`, `TypeError` | A value KivaDB cannot store (a naive datetime, an object...), before anything is sent |

## Connections

* **One client is one session:** its workspace, its transaction, its subscriptions. Requests from several threads are sent one at a time on it. For transactions, use one client per thread.
* **Lost connection.** The request that was being sent fails with `KivaConnectionError` and is never sent again by the driver: it may have been applied. The next request opens a new connection, logs in again and goes back to the workspace given to `connect`. A transaction open at that moment is lost (the server rolled it back), and the `with` block raises `KivaConnectionError` saying so. `reconnect=False` turns reconnecting off.
* **Timeout.** After a `KivaTimeoutError`, the connection is closed, because whether the request was applied is not known. `changes(wait=...)` waits longer than `timeout` when it has to.
* `close()`, or the end of a `with kivadb.connect(...)` block, closes the connection. The server rolls back a transaction left open.

## Tests

Against a real server, started for the tests:

```bash
cargo build
cd drivers/python
python -m unittest discover -s tests -v
```

`KIVADB_BIN` names another executable. The continuous integration runs them on Windows, Linux and macOS with Python 3.10, 3.12 and 3.14, and checks the types with `mypy --strict`.
