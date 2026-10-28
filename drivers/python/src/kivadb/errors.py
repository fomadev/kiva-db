"""The errors the driver raises."""

from __future__ import annotations

import re
from typing import Optional

_CODE = re.compile(r"^(?:Error: )?([A-Z][A-Z_]+): ")


class KivaError(Exception):
    """An error KivaDB reported, or one of the driver.

    ``code`` is the word before the colon of KivaDB's message
    (``"UNIQUE"``, ``"SCHEMA"``, ``"CURSOR_EXPIRED"``...), or ``None``.
    """

    def __init__(self, message: str) -> None:
        super().__init__(message)
        self.message = message
        m = _CODE.match(message)
        self.code: Optional[str] = m.group(1) if m else None


class AuthenticationError(KivaError):
    """The user or the password was refused."""


class KivaConnectionError(KivaError, ConnectionError):
    """The server could not be reached, or the connection was lost.

    A request sent when the connection broke may or may not have been
    applied: the driver never sends it again by itself.
    """


class KivaTimeoutError(KivaError, TimeoutError):
    """No answer within the timeout. The connection is closed: what the
    request did is not known."""
