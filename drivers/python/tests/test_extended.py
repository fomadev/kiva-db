"""Native values with ``types="extended"`` (docs/architecture/NATIVE_DOCUMENTS.md,
section 23), against a real KivaDB server started for the tests.

Run with ``python -m unittest discover -s tests`` from drivers/python.
"""

from __future__ import annotations

import datetime as dt
import socket
import struct
import sys
import threading
import unittest
from decimal import Decimal
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "src"))
sys.path.insert(0, str(HERE))

import kivadb  # noqa: E402
from kivadb import KivaError  # noqa: E402
from kivadb.values import condition, decimal_text, loads_extended, to_extended  # noqa: E402
from test_driver import BINARY, Server  # noqa: E402

UTC = dt.timezone.utc
ACCOUNT = (
    'set acc:1 {"name": "Ana", "balance": decimal "1250.50", "opened": date "2026-01-15", '
    '"at": datetime "2026-10-31T15:18:00Z", "alarm": time "08:30:00", "rate": float 8}'
)


class ValuesTest(unittest.TestCase):
    def test_documents_are_written_with_tags_and_doubled_dollars(self) -> None:
        doc = {
            "$ref": 1,
            "price": Decimal("9.90"),
            "when": dt.datetime(2026, 2, 2, 10, 0, 0, 250000, tzinfo=UTC),
            "day": dt.date(2026, 1, 1),
            "alarm": dt.time(8, 30),
            "list": (Decimal("1E+2"), 2),
        }
        self.assertEqual(
            to_extended(doc),
            {
                "$$ref": 1,
                "price": {"$decimal": "9.90"},
                "when": {"$datetime": "2026-02-02T10:00:00.250Z"},
                "day": {"$date": "2026-01-01"},
                "alarm": {"$time": "08:30:00"},
                "list": [{"$decimal": "100"}, 2],
            },
        )

    def test_what_kivadb_cannot_hold_is_refused(self) -> None:
        with self.assertRaisesRegex(ValueError, "finite"):
            decimal_text(Decimal("NaN"))
        with self.assertRaisesRegex(ValueError, "finite"):
            decimal_text(Decimal("-Infinity"))
        with self.assertRaisesRegex(ValueError, "more digits"):
            decimal_text(Decimal("1." + "1" * 28))
        with self.assertRaisesRegex(ValueError, r"decimal.Decimal\('9223372036854775808'\)"):
            to_extended({"n": 2**63})
        with self.assertRaisesRegex(ValueError, "naive datetime"):
            to_extended([dt.datetime(2026, 1, 1)])
        with self.assertRaisesRegex(TypeError, "bytes cannot go inside a document"):
            to_extended({"b": b"x"})

    def test_answers_are_read_with_their_types(self) -> None:
        text = (
            '{"a": {"$decimal": "1.50"}, "d": {"$date": "2026-01-15"}, "t": {"$time": "08:30:00"},'
            ' "w": {"$datetime": "2026-10-31T15:18:00.000Z"}, "$$k": {"$$decimal": "x"}, "n": 9007199254740993}'
        )
        self.assertEqual(
            loads_extended(text),
            {
                "a": Decimal("1.50"),
                "d": dt.date(2026, 1, 15),
                "t": dt.time(8, 30),
                "w": dt.datetime(2026, 10, 31, 15, 18, tzinfo=UTC),
                "$k": {"$decimal": "x"},
                "n": 9007199254740993,
            },
        )

    def test_conditions_tag_native_values(self) -> None:
        where = {"and": [{"field": "balance", "cmp": "in", "value": [Decimal("1.5")]}, {"day": dt.date(2026, 1, 1)}]}
        self.assertEqual(
            condition(where),
            {"and": [{"field": "balance", "cmp": "in", "value": [{"$decimal": "1.5"}]}, {"day": {"$date": "2026-01-01"}}]},
        )


class OldServerTest(unittest.TestCase):
    def test_extended_mode_needs_a_2_0_server(self) -> None:
        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        port = listener.getsockname()[1]

        def serve() -> None:
            for _ in range(2):
                conn, _ = listener.accept()
                with conn:
                    if conn.recv(64):
                        payload = b"KivaDB 1.7.1\nauth=none\nuser=kiva"
                        conn.sendall(struct.pack(">IH", len(payload) + 2, 0) + payload)
                        conn.recv(64)

        thread = threading.Thread(target=serve, daemon=True)
        thread.start()
        try:
            with self.assertRaisesRegex(KivaError, r"needs KivaDB 2\.0\.0 or later; .* is KivaDB 1\.7\.1"):
                kivadb.connect(port=port, types="extended", reconnect=False)
            db = kivadb.connect(port=port, reconnect=False)
            self.assertEqual(db.server, "KivaDB 1.7.1")
            db.close()
        finally:
            listener.close()
        with self.assertRaisesRegex(ValueError, 'types is "plain" or "extended"'):
            kivadb.connect(port=port, types="typed")


@unittest.skipUnless(BINARY, "KivaDB is not built (set KIVADB_BIN)")
class ExtendedTest(unittest.TestCase):
    server: Server

    @classmethod
    def setUpClass(cls) -> None:
        cls.server = Server()

    @classmethod
    def tearDownClass(cls) -> None:
        cls.server.remove()

    def connect(self, types: str = "plain") -> kivadb.Client:
        db = kivadb.connect(port=self.server.port, types=types)
        self.addCleanup(db.close)
        return db

    def test_documents_round_trip_with_every_type(self) -> None:
        plain, ext = self.connect(), self.connect("extended")
        plain.exec(ACCOUNT.replace("acc:1", "doc:1"))
        self.assertEqual(plain.get("doc:1")["balance"], "1250.50")
        got = ext.get("doc:1")
        self.assertEqual(got["balance"], Decimal("1250.50"))
        self.assertEqual(str(got["balance"]), "1250.50")
        self.assertEqual(got["opened"], dt.date(2026, 1, 15))
        self.assertEqual(got["alarm"], dt.time(8, 30))
        self.assertEqual(got["at"], dt.datetime(2026, 10, 31, 15, 18, tzinfo=UTC))
        self.assertIsInstance(got["rate"], float)

        doc = {
            "price": Decimal("9.90"),
            "when": dt.datetime(2026, 2, 2, 11, 0, 0, 250000, tzinfo=dt.timezone(dt.timedelta(hours=1))),
            "day": dt.date(2026, 3, 1),
            "big": 2**62,
            "$ref": "kept",
        }
        ext.set("item:1", doc)
        self.assertEqual(plain.exec("json type item:1 .price"), ["item:1.price: [decimal]"])
        self.assertEqual(plain.exec("json type item:1 .day"), ["item:1.day: [date]"])
        back = ext.get("item:1")
        self.assertEqual(back, {**doc, "when": dt.datetime(2026, 2, 2, 10, 0, 0, 250000, tzinfo=UTC)})
        self.assertEqual(plain.api({"op": "get", "key": "item:1"})["value"]["$ref"], "kept")
        key, _ = ext.add("item:", {"d": dt.date(2026, 1, 1)})
        self.assertEqual(ext.get(key), {"d": dt.date(2026, 1, 1)})

        # Plain mode is unchanged: moments inside a document are their texts.
        plain.set("p", {"day": dt.date(2026, 1, 1)})
        self.assertEqual(plain.exec("json type p .day"), ["p.day: [string]"])

    def test_decimal_keys_and_plain_refusal(self) -> None:
        plain, ext = self.connect(), self.connect("extended")
        ext.set("d", Decimal("0.10"))
        self.assertEqual(plain.exec("typeof d"), [" -> d is a [decimal]"])
        self.assertEqual(ext.get("d"), Decimal("0.10"))
        self.assertEqual(plain.get("d"), "0.10")
        with self.assertRaisesRegex(TypeError, 'types="extended"'):
            plain.set("d2", Decimal("1"))
        with self.assertRaisesRegex(ValueError, "64-bit"):
            ext.set("doc", {"n": 2**64})

    def test_conditions_and_the_change_journal(self) -> None:
        plain, ext = self.connect(), self.connect("extended")
        plain.exec(ACCOUNT)
        plain.exec('set acc:2 {"name": "Ben", "balance": decimal "0.10", "opened": date "2025-01-01"}')
        for indexed in (False, True):
            if indexed:
                plain.exec("create index on acc:*.balance")
            for db in (plain, ext):
                rows = db.query("acc:*", {"field": "balance", "cmp": "=", "value": Decimal("1250.5")})
                self.assertEqual([k for k, _ in rows], ["acc:1"])
                rows = db.query("acc:*", {"field": "opened", "cmp": "<", "value": dt.date(2026, 1, 1)})
                self.assertEqual([k for k, _ in rows], ["acc:2"])
        rows = ext.query("acc:*", {"name": "Ben"})
        self.assertEqual(rows[0][1]["balance"], Decimal("0.10"))
        total = ext.aggregate("acc:*", {"n": "count"}, where={"field": "balance", "cmp": ">", "value": Decimal("1")})
        self.assertEqual(total["values"]["n"], 1, total)

        plain.exec("changes enable")
        ext.set("c", {"v": Decimal("2.50")})
        changes = ext.changes("beginning")["changes"]
        self.assertEqual(changes[-1]["value"], {"v": Decimal("2.50")})
        self.assertEqual(plain.changes("beginning")["changes"][-1]["value"], {"v": "2.50"})


if __name__ == "__main__":
    unittest.main()
