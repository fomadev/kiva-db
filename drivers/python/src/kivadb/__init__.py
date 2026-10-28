"""Official Python driver for KivaDB (binary protocol, no dependencies).

>>> import kivadb
>>> with kivadb.connect(password="...") as db:
...     db.set("user:1", {"name": "Ana", "role": "dev"})
...     db.get("user:1")
{'name': 'Ana', 'role': 'dev'}
"""

from .client import Change, Client, connect
from .errors import AuthenticationError, KivaConnectionError, KivaError, KivaTimeoutError

__version__ = "1.0.0"

__all__ = [
    "AuthenticationError",
    "Change",
    "Client",
    "KivaConnectionError",
    "KivaError",
    "KivaTimeoutError",
    "connect",
    "__version__",
]
