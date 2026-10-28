"""The driver against a real KivaDB server started for the tests.

The executable comes from KIVADB_BIN, or from ../../target/{debug,release}.
Run with ``python -m unittest discover -s tests`` from drivers/python.
"""

from __future__ import annotations

import datetime as dt
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from typing import Dict, List, Optional

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))

import kivadb  # noqa: E402
from kivadb import AuthenticationError, KivaConnectionError, KivaError, KivaTimeoutError  # noqa: E402

ROOT = HERE.parent.parent.parent
EXE = "kivadb.exe" if os.name == "nt" else "kivadb"
CANDIDATES = [os.environ.get("KIVADB_BIN"), ROOT / "target" / "debug" / EXE, ROOT / "target" / "release" / EXE]
BINARY: Optional[str] = next((str(p) for p in CANDIDATES if p and Path(p).exists()), None)
KINSHASA = dt.timezone(dt.timedelta(hours=1))


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port: int = s.getsockname()[1]
        return port


class Server:
    """A KivaDB of its own, in a temporary home."""

    def __init__(self) -> None:
        assert BINARY
        self.home = tempfile.mkdtemp(prefix="kivadb-py-")
        self.port = free_port()
        self.proc: Optional[subprocess.Popen[bytes]] = None
        self.start()

    def start(self) -> None:
        assert BINARY
        args = [BINARY, "serve", "--home", self.home, "--port", str(self.port), "--no-http", "--no-resp"]
        self.proc = subprocess.Popen(args, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        for _ in range(200):
            try:
                socket.create_connection(("127.0.0.1", self.port), timeout=0.2).close()
                return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError("KivaDB did not start")

    def stop(self) -> None:
        if self.proc and self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait()

    def remove(self) -> None:
        self.stop()
        shutil.rmtree(self.home, ignore_errors=True)


@unittest.skipUnless(BINARY, "KivaDB is not built (set KIVADB_BIN)")
class DriverTest(unittest.TestCase):
    server: Server

    @classmethod
    def setUpClass(cls) -> None:
        cls.server = Server()

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.remove()

    def connect(self, **kw: object) -> kivadb.Client:
        db = kivadb.connect(port=self.server.port, **kw)  # type: ignore[arg-type]
        self.addCleanup(db.close)
        return db

    def test_the_server_answers_with_its_version(self) -> None:
        db = self.connect()
        cargo = (ROOT / "Cargo.toml").read_text()
        version = re.search(r'^version = "(.+)"', cargo, re.M)
        assert version
        self.assertEqual(db.server, f"KivaDB {version.group(1)}")
        db.ping()

    def test_typed_values_round_trip(self) -> None:
        db = self.connect()
        values = {
            "s": "both \" and ' quotes, é",
            "i": 2**53 + 1,
            "f": 3.5,
            "b": True,
            "z": None,
            "list": [1, "two", None],
            "doc": {"name": "Ana", "tags": ["a"], "n": {"deep": 1}},
            "raw": b"\x00\x01\xff",
            "born": dt.date(2004, 5, 9),
            "opens": dt.time(8, 0),
            "late": dt.time(23, 59, 59, 500000),
        }
        for k, v in values.items():
            db.set(k, v)
        for k, v in values.items():
            self.assertEqual(db.get(k), v, k)
        self.assertIsInstance(db.get("b"), bool)
        self.assertEqual(db.get("ghost"), None)
        self.assertEqual(db.get("ghost", "default"), "default")
        self.assertEqual(db.exec("get born and opens"), ["born: 2004-05-09 [date]", "opens: 08:00:00 [time]"])
        # A tuple is an array; bytearray and memoryview are bytes.
        db.set("t", (1, 2))
        self.assertEqual(db.get("t"), [1, 2])
        db.set("ba", bytearray(b"ab"))
        self.assertEqual(db.get("ba"), b"ab")

    def test_datetimes_are_instants_and_naive_ones_are_refused(self) -> None:
        db = self.connect()
        at = dt.datetime(2026, 10, 27, 14, 30, 0, 250999, tzinfo=KINSHASA)
        db.set("at", at)
        back = db.get("at")
        # The same instant, in UTC, to the millisecond (KivaDB keeps milliseconds).
        self.assertEqual(back, dt.datetime(2026, 10, 27, 13, 30, 0, 250000, tzinfo=dt.timezone.utc))
        self.assertEqual(back.utcoffset(), dt.timedelta(0))
        db.exec("timezone UTC")
        self.assertEqual(db.exec("get at"), ["at: 2026-10-27 13:30:00.250 +00:00 [datetime]"])
        with self.assertRaisesRegex(ValueError, "naive datetime"):
            db.set("bad", dt.datetime(2026, 10, 27, 14, 30))
        self.assertIsNone(db.get("bad"))
        with self.assertRaisesRegex(ValueError, "has no zone"):
            db.set("bad", dt.time(8, 0, tzinfo=dt.timezone.utc))
        with self.assertRaisesRegex(KivaError, "is not a date"):
            db.set("bad", "27/10/2026", type="date")
        db.set("text-date", "2026-10-27", type="date")
        self.assertEqual(db.get("text-date"), dt.date(2026, 10, 27))
        # Inside a document, moments are ISO 8601 text, compared by KivaDB as moments.
        db.set("ev:1", {"at": at, "day": dt.date(2026, 10, 27), "opens": dt.time(8, 0)})
        self.assertEqual(db.get("ev:1"), {"at": "2026-10-27T13:30:00.250Z", "day": "2026-10-27", "opens": "08:00:00"})
        db.set("ev:2", {"at": dt.datetime(2026, 10, 27, 12, 0, tzinfo=dt.timezone.utc)})
        rows = db.query("ev:*", {"field": "at", "cmp": ">", "value": {"$datetime": "2026-10-27T13:00:00Z"}})
        self.assertEqual([k for k, _ in rows], ["ev:1"])
        with self.assertRaisesRegex(ValueError, "naive datetime"):
            db.set("ev:3", {"at": dt.datetime(2026, 1, 1)})
        with self.assertRaisesRegex(TypeError, "bytes cannot go inside a document"):
            db.set("ev:3", {"b": b"x"})
        with self.assertRaises(TypeError):
            db.set("ev:3", object())

    def test_modes_deletes_ttl_and_error_codes(self) -> None:
        db = self.connect()
        db.set("m", "x")
        with self.assertRaises(KivaError):
            db.set("m", "y", mode="create")
        with self.assertRaisesRegex(KivaError, "not found"):
            db.set("m-ghost", "y", mode="update")
        self.assertTrue(db.delete("m"))
        self.assertFalse(db.delete("m"))
        db.set("ttl", 1, ttl=60)
        keys = db.scan("ttl")
        self.assertEqual(keys[0]["key"], "ttl")
        self.assertGreater(keys[0]["ttl"], 50)
        db.exec("create unique index on mail: (address)")
        db.set("mail:1", {"address": "a@x"})
        with self.assertRaises(KivaError) as e:
            db.set("mail:2", {"address": "a@x"})
        self.assertEqual(e.exception.code, "UNIQUE")
        db.exec('schema set pet: {"name": {"type": "string", "required": true}}')
        with self.assertRaises(KivaError) as e:
            db.set("pet:1", {"age": 3})
        self.assertEqual(e.exception.code, "SCHEMA")
        db.set("num", 3)
        with self.assertRaisesRegex(KivaError, r"Cannot update \[number\]"):
            db.exec('update num "text"')
        key, ident = db.add("place:", {"name": "Goma"})
        self.assertRegex(ident, r"^[0-9a-f]{24}$")
        self.assertEqual(key, "place:" + ident)

    def test_queries_order_pages_explain_and_aggregates(self) -> None:
        db = self.connect()
        for i, (age, team) in enumerate([(30, "red"), (25, "blue"), (41, "red"), (19, "blue"), (35, None)]):
            doc: Dict[str, object] = {"age": age, "n": i}
            if team:
                doc["team"] = team
            db.set(f"q:{i}", doc)
        rows = db.query("q:*", {"field": "age", "cmp": ">", "value": 20}, order_by="age desc", limit=2)
        self.assertEqual([k for k, _ in rows], ["q:2", "q:4"])
        page = db.query("q:*", {"field": "age", "cmp": ">", "value": 20}, order_by="age desc", limit=2, after="q:4")
        self.assertEqual([k for k, _ in page], ["q:0", "q:1"])
        self.assertEqual([k for k, _ in db.query("q:*", {"team": "blue"}, reverse=True)], ["q:3", "q:1"])
        db.exec("create index on q:* (age)")
        plan = db.explain("q:*", {"field": "age", "cmp": ">", "value": 20}, order_by="age desc", limit=2)
        self.assertEqual(plan["plan"]["strategy"], "INDEX ORDER SCAN")
        self.assertNotIn("analyze", plan)
        measured = db.explain("q:*", {"field": "age", "cmp": ">", "value": 20}, analyze=True)
        self.assertEqual(measured["analyze"]["returned"], 4)
        self.assertEqual(measured["analyze"]["documents_read"], 4)
        agg = db.aggregate("q:*", {"n": "count", "mean": {"avg": "age"}}, group_by="team")
        groups = {g["key"]: g["values"]["n"] for g in agg["groups"]}
        self.assertEqual(groups, {"blue": 2, "red": 2, None: 1})
        with self.assertRaisesRegex(KivaError, "sorting pipelines"):
            db.api({"op": "query", "target": "q:*", "where": {"n": 1}, "offset": 2})

    def test_transactions_commit_together_or_not_at_all(self) -> None:
        db = self.connect()
        other = self.connect()
        with db.transaction():
            db.set("acct:a", 10)
            db.set("acct:b", 20)
            # Not visible to another session before the commit.
            self.assertIsNone(other.get("acct:a"))
        self.assertEqual((other.get("acct:a"), other.get("acct:b")), (10, 20))
        with self.assertRaisesRegex(RuntimeError, "boom"):
            with db.transaction():
                db.set("acct:a", 0)
                db.delete("acct:b")
                raise RuntimeError("boom")
        self.assertEqual((other.get("acct:a"), other.get("acct:b")), (10, 20))
        db.exec("create unique index on user: (email)")
        db.set("user:1", {"email": "x@y"})
        with self.assertRaises(KivaError) as e:
            with db.transaction():
                db.set("user:2", {"email": "z@y"})
                db.set("user:3", {"email": "x@y"})
        self.assertEqual(e.exception.code, "UNIQUE")
        self.assertIsNone(other.get("user:2"), "a refused commit writes nothing")
        with db.transaction():
            with self.assertRaisesRegex(KivaError, "already open"):
                with db.transaction():
                    pass
        db.set("after", 1)
        self.assertEqual(other.get("after"), 1, "the session is out of the transaction")

    def test_workspaces_and_authentication(self) -> None:
        admin = self.connect()
        admin.exec("workspace create shop")
        admin.exec('create user "app" identified by "pw" grant write on shop')
        app = self.connect(user="app", password="pw", workspace="shop")
        app.set("item:1", 3)
        self.assertIsNone(admin.get("item:1"), "another workspace")
        self.assertEqual(self.connect(workspace="shop").get("item:1"), 3)
        with self.assertRaises(AuthenticationError):
            kivadb.connect(port=self.server.port, user="app", password="wrong")
        with self.assertRaisesRegex(KivaError, "password is required"):
            kivadb.connect(host="203.0.113.7")
        with self.assertRaises(ValueError):
            kivadb.connect(port=self.server.port, workspace="a b")
        with self.assertRaisesRegex(KivaError, "no access"):
            kivadb.connect(port=self.server.port, user="app", password="pw", workspace="default")

    def test_the_change_journal(self) -> None:
        db = self.connect()
        db.exec("workspace create journal")
        db.exec("workspace switch journal")
        db.exec("changes enable")
        db.set("o:1", {"total": 5})
        with db.transaction():
            db.set("o:2", {"total": 7})
            db.delete("o:1")
        r = db.changes("beginning")
        # The writes of a transaction come together, in the order it commits them (by key).
        self.assertEqual([(c["op"], c["key"]) for c in r["changes"]], [("set", "o:1"), ("delete", "o:1"), ("set", "o:2")])
        self.assertEqual([c["txn_end"] for c in r["changes"]], [False, False, True])
        cursor = r["cursor"]
        self.assertEqual(db.changes(cursor)["changes"], [])

        def later() -> None:
            time.sleep(0.4)
            with kivadb.connect(port=self.server.port, workspace="journal") as w:
                w.set("o:3", 1)

        threading.Thread(target=later).start()
        started = time.monotonic()
        waited = db.changes(cursor, wait=10)
        self.assertEqual([c["key"] for c in waited["changes"]], ["o:3"])
        self.assertLess(time.monotonic() - started, 8)
        with self.assertRaises(KivaError) as e:
            db.changes("0" * 32)
        self.assertEqual(e.exception.code, "CURSOR_INVALID")

    def test_notifications(self) -> None:
        db = self.connect()
        db.subscribe("order:*")
        writer = self.connect()
        writer.set("order:1", {"total": 5})
        writer.delete("order:1")
        writer.set("other", 1)
        seen = [db.next_change(timeout=5), db.next_change(timeout=5)]
        self.assertEqual([(c.event, c.key, c.type) for c in seen if c], [("set", "order:1", "object"), ("del", "order:1", None)])
        self.assertIsNone(db.next_change(timeout=0.3))

    def test_threads_share_a_client_one_request_at_a_time(self) -> None:
        db = self.connect()
        errors: List[BaseException] = []

        def work(t: int) -> None:
            try:
                for i in range(30):
                    db.set(f"th:{t}:{i}", i)
                    assert db.get(f"th:{t}:{i}") == i
            except BaseException as e:  # noqa: BLE001
                errors.append(e)

        threads = [threading.Thread(target=work, args=(t,)) for t in range(6)]
        for th in threads:
            th.start()
        for th in threads:
            th.join()
        self.assertEqual(errors, [])
        self.assertEqual(len(db.scan("th:*", limit=1000)), 180)

    def test_timeouts(self) -> None:
        # A server that accepts the connection and never answers.
        silent = socket.socket()
        silent.bind(("127.0.0.1", 0))
        silent.listen(1)
        self.addCleanup(silent.close)
        started = time.monotonic()
        with self.assertRaises(KivaTimeoutError):
            kivadb.connect(port=silent.getsockname()[1], timeout=0.5)
        self.assertLess(time.monotonic() - started, 5)
        # Nothing listening: refused, or on Windows unanswered until the timeout.
        with self.assertRaises((KivaConnectionError, KivaTimeoutError)):
            kivadb.connect(port=free_port(), timeout=2)


@unittest.skipUnless(BINARY, "KivaDB is not built (set KIVADB_BIN)")
class RestartTest(unittest.TestCase):
    def test_a_lost_connection_fails_once_then_reconnects(self) -> None:
        server = Server()
        self.addCleanup(server.remove)
        db = kivadb.connect(port=server.port, timeout=3)
        self.addCleanup(db.close)
        db.exec("workspace create kept")
        db.exec("workspace switch kept")
        db.set("k", 1)
        db.workspace = "kept"
        server.stop()
        server.start()
        # The request sent on the broken connection fails, and is not sent again.
        with self.assertRaises(KivaConnectionError):
            db.set("k", 2)
        self.assertFalse(db.connected)
        # The next one reconnects, in the same workspace.
        self.assertEqual(db.get("k"), 1)
        # A transaction does not survive: the server rolled it back.
        with self.assertRaises(KivaConnectionError):
            with db.transaction():
                db.set("k", 3)
                server.stop()
                server.start()
                db.set("k", 4)
        self.assertEqual(db.get("k"), 1)
        db.close()
        with self.assertRaisesRegex(KivaConnectionError, "Not connected"):
            db.ping()


@unittest.skipUnless(BINARY, "KivaDB is not built (set KIVADB_BIN)")
class TlsTest(unittest.TestCase):
    def test_tls_with_the_fingerprint_of_the_server(self) -> None:
        assert BINARY
        server = Server()
        self.addCleanup(server.remove)
        shown = subprocess.run([BINARY, "--home", server.home, "tls"], capture_output=True, text=True, check=True).stdout
        m = re.search(r"SHA-256\s*:\s*(\S+)", shown)
        assert m, shown
        fingerprint = m.group(1)
        with kivadb.connect(port=server.port, tls={"fingerprint": fingerprint}) as db:
            db.set("secret", "encrypted")
            self.assertEqual(db.get("secret"), "encrypted")
        # The self-signed certificate is not trusted without its fingerprint.
        with self.assertRaises(KivaConnectionError):
            kivadb.connect(port=server.port, tls=True)
        with self.assertRaisesRegex(KivaConnectionError, "not the expected one"):
            kivadb.connect(port=server.port, tls={"fingerprint": "AB:" * 31 + "AB"})


if __name__ == "__main__":
    unittest.main()
