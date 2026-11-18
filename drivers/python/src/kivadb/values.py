"""Python values and KivaDB types, both ways.

=====================  ============  ========================================
Python                 KivaDB        Back in Python
=====================  ============  ========================================
``str``                ``string``    ``str``
``int``, ``float``     ``number``    ``int`` or ``float``
``bool``               ``boolean``   ``bool``
``None``               ``null``      ``None``
``list``, ``tuple``    ``array``     ``list``
``dict``               ``object``    ``dict``
``bytes``              ``bytes``     ``bytes``
``datetime`` (aware)   ``datetime``  ``datetime`` in UTC, to the millisecond
``date``               ``date``      ``date``
``time`` (naive)       ``time``      ``time``
``Decimal``            ``decimal``   ``Decimal`` (``types="extended"`` only)
=====================  ============  ========================================

A naive ``datetime`` (without ``tzinfo``) is refused: it would mean another
instant on every machine. Give it a zone (``datetime.timezone.utc``,
``zoneinfo.ZoneInfo("Africa/Kinshasa")``). Inside a document (a ``dict`` or
a ``list``), datetimes, dates and times are written as ISO 8601 text, the
form KivaDB compares as moments, and come back as text.

With ``connect(..., types="extended")`` (KivaDB 2.0.0 and later), every
type is kept inside documents too: ``Decimal``, ``date``, ``time`` and
aware ``datetime`` are stored as native values and come back as themselves,
and a ``Decimal`` can be a key's value. In conditions, these objects are
compared exactly in both modes.
"""

from __future__ import annotations

import base64
import datetime as dt
import json
from decimal import Decimal
from typing import Any, Dict, Optional

NAIVE = (
    "a naive datetime has no time zone, so it means another instant on every machine: "
    "give it one, e.g. value.replace(tzinfo=datetime.timezone.utc) or "
    "value.astimezone() for the machine's own"
)


def type_of(value: Any, extended: bool = False) -> str:
    """The KivaDB type a Python value is stored as."""
    if isinstance(value, Decimal):
        if not extended:
            raise TypeError('a Decimal is stored exactly with connect(..., types="extended")')
        return "decimal"
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "boolean"
    if isinstance(value, (int, float)):
        return "number"
    if isinstance(value, str):
        return "string"
    if isinstance(value, (bytes, bytearray, memoryview)):
        return "bytes"
    if isinstance(value, dt.datetime):
        return "datetime"
    if isinstance(value, dt.date):
        return "date"
    if isinstance(value, dt.time):
        return "time"
    if isinstance(value, (list, tuple)):
        return "array"
    if isinstance(value, dict):
        return "object"
    raise TypeError(f"KivaDB cannot store a {type(value).__name__}")


def iso_datetime(value: dt.datetime) -> str:
    if value.tzinfo is None or value.utcoffset() is None:
        raise ValueError(NAIVE)
    utc = value.astimezone(dt.timezone.utc)
    return utc.strftime("%Y-%m-%dT%H:%M:%S.") + f"{utc.microsecond // 1000:03d}Z"


def iso_time(value: dt.time) -> str:
    if value.tzinfo is not None:
        raise ValueError("a KivaDB time has no zone: give a naive time (value.replace(tzinfo=None))")
    return value.isoformat()


def _in_document(value: Any) -> Any:
    if isinstance(value, dt.datetime):
        return iso_datetime(value)
    if isinstance(value, dt.date):
        return value.isoformat()
    if isinstance(value, dt.time):
        return iso_time(value)
    if isinstance(value, (bytes, bytearray, memoryview)):
        raise TypeError("bytes cannot go inside a document: store them under a key of their own, or as base64 text")
    raise TypeError(f"a {type(value).__name__} cannot go inside a document")


def document(value: Any) -> Any:
    """A list or dict as JSON can carry it, moments written as ISO text."""
    return json.loads(json.dumps(value, default=_in_document))


def to_wire(kiva_type: str, value: Any, extended: bool = False) -> Any:
    """The value as a structured request carries it."""
    if kiva_type == "decimal" and isinstance(value, Decimal):
        return {"$decimal": decimal_text(value)}
    if extended and kiva_type in ("array", "object"):
        return to_extended(value)
    if kiva_type == "bytes":
        return base64.b64encode(bytes(value)).decode("ascii")
    if kiva_type == "datetime":
        if isinstance(value, dt.datetime):
            return iso_datetime(value)
        return value
    if kiva_type == "date" and isinstance(value, dt.date):
        if isinstance(value, dt.datetime):
            raise ValueError("a date is a day: give datetime.date, or value.date()")
        return value.isoformat()
    if kiva_type == "time" and isinstance(value, dt.time):
        return iso_time(value)
    if kiva_type in ("array", "object"):
        return document(list(value) if isinstance(value, tuple) else value)
    return value


def parse_datetime(text: str) -> dt.datetime:
    """ISO 8601 as KivaDB gives it (``2026-10-27T13:30:00.250Z``), in UTC."""
    return dt.datetime.fromisoformat(text.replace("Z", "+00:00")).astimezone(dt.timezone.utc)


def from_wire(kiva_type: Optional[str], value: Any) -> Any:
    if kiva_type == "bytes":
        return base64.b64decode(value)
    if kiva_type == "datetime" and isinstance(value, str):
        return parse_datetime(value)
    if kiva_type == "date" and isinstance(value, str):
        return dt.date.fromisoformat(value)
    if kiva_type == "time" and isinstance(value, str):
        return dt.time.fromisoformat(value)
    return value


# -- extended JSON (docs/architecture/NATIVE_DOCUMENTS.md, section 23) -------

I64_MIN, I64_MAX = -(2**63), 2**63 - 1

#: The digits a KivaDB decimal holds.
DECIMAL_DIGITS = 28


def decimal_text(value: Decimal) -> str:
    """A ``Decimal`` as KivaDB reads it: its digits, without an exponent."""
    if not value.is_finite():
        raise ValueError("a KivaDB decimal is finite: NaN and infinities are refused")
    text = format(value, "f")
    if sum(c.isdigit() for c in text) > DECIMAL_DIGITS:
        raise ValueError(f"{text} has more digits than a KivaDB decimal holds ({DECIMAL_DIGITS})")
    return text


def _exact_int(value: int) -> int:
    if not I64_MIN <= value <= I64_MAX:
        raise ValueError(
            f"{value} is beyond the 64-bit integers of KivaDB: store it exactly as decimal.Decimal({str(value)!r})"
        )
    return value


def to_extended(value: Any) -> Any:
    """A document as extended JSON carries it: native values tagged, the
    ``$`` of its keys doubled."""
    if value is None or isinstance(value, (bool, str, float)):
        return value
    if isinstance(value, int):
        return _exact_int(value)
    if isinstance(value, Decimal):
        return {"$decimal": decimal_text(value)}
    if isinstance(value, dt.datetime):
        return {"$datetime": iso_datetime(value)}
    if isinstance(value, dt.date):
        return {"$date": value.isoformat()}
    if isinstance(value, dt.time):
        return {"$time": iso_time(value)}
    if isinstance(value, dict):
        return {("$" + k if isinstance(k, str) and k.startswith("$") else k): to_extended(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [to_extended(v) for v in value]
    return _in_document(value)


def condition(value: Any) -> Any:
    """A condition with its native values tagged, in both modes: a
    ``Decimal``, ``date``, ``time`` or aware ``datetime`` compares exactly."""
    if isinstance(value, Decimal):
        return {"$decimal": decimal_text(value)}
    if isinstance(value, dt.datetime):
        return {"$datetime": iso_datetime(value)}
    if isinstance(value, dt.date):
        return {"$date": value.isoformat()}
    if isinstance(value, dt.time):
        return {"$time": iso_time(value)}
    if isinstance(value, dict):
        return {k: condition(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [condition(v) for v in value]
    return value


def _from_tags(m: Dict[str, Any]) -> Any:
    if len(m) == 1:
        ((tag, text),) = m.items()
        if isinstance(text, str):
            if tag == "$decimal":
                return Decimal(text)
            if tag == "$date":
                return dt.date.fromisoformat(text)
            if tag == "$time":
                return dt.time.fromisoformat(text)
            if tag == "$datetime":
                return parse_datetime(text)
    return {(k[1:] if k.startswith("$$") else k): v for k, v in m.items()}


def loads_extended(text: str) -> Any:
    """An answer in extended JSON, its native values as Python values."""
    return json.loads(text, object_hook=_from_tags)
