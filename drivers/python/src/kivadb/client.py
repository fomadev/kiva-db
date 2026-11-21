"""The connection to a KivaDB server, over its binary protocol (port 2911)."""

from __future__ import annotations

import collections
import hashlib
import ipaddress
import json
import re
import socket
import ssl
import struct
import threading
import time
from contextlib import contextmanager
from typing import Any, Deque, Dict, Iterator, List, Mapping, Optional, Sequence, Tuple, Union

from .errors import AuthenticationError, KivaConnectionError, KivaError, KivaTimeoutError
from .values import condition, from_wire, loads_extended, to_wire, type_of

_HELLO, _AUTH, _EXEC, _PING, _API = 1, 2, 3, 4, 8
_OK, _ERROR, _NOT_FOUND, _BYE, _EVENT = 0, 1, 2, 3, 9
_HEAD = struct.Struct(">IH")
_MAX_FRAME = 300 * 1024 * 1024
_WORKSPACE = re.compile(r"^[A-Za-z0-9_.-]{1,64}$")

TLS = Union[bool, ssl.SSLContext, Mapping[str, str], None]


def _is_local(host: str) -> bool:
    h = host.strip().lower().strip("[]")
    if h == "localhost":
        return True
    try:
        return ipaddress.ip_address(h).is_loopback
    except ValueError:
        return False


def _same_fingerprint(seen: str, wanted: str) -> bool:
    def norm(s: str) -> str:
        s = re.sub(r"^(sha256/|SHA256:)", "", s.strip())
        return s.replace(":", "").lower()

    return len(norm(wanted)) == 64 and norm(seen) == norm(wanted)


class Change:
    """A notification from :meth:`Client.subscribe`: ``event`` is ``set``,
    ``del``, ``expire`` or ``reset``, ``key`` the key (``None`` for a reset)."""

    __slots__ = ("event", "key", "type", "raw")

    def __init__(self, raw: str) -> None:
        parts = raw.split(" ")
        self.event = parts[0]
        self.key: Optional[str] = None if self.event == "reset" or len(parts) < 2 else parts[1]
        self.type: Optional[str] = parts[2].strip("()") if len(parts) > 2 and self.event != "reset" else None
        self.raw = raw

    def __repr__(self) -> str:
        return f"Change({self.raw!r})"


class Client:
    """One connection, and the session it carries: its collection, its
    transaction, its subscriptions.

    ``collection`` (KivaDB 2.0.0-rc.5 calls workspaces collections) is the
    one the client works in: ``"travel.places"``, or a name outside a
    project. ``workspace`` is the same option under its former name.

    Use one client per thread for transactions: requests from several
    threads are sent one at a time, but share the session. The client is a
    context manager, closed at the end of a ``with`` block.

    When the connection is lost, the request that was being sent fails with
    :class:`KivaConnectionError` and is not sent again (it may have been
    applied). The next request opens a new connection, logs in and goes back
    to the collection, unless a transaction was open: it is lost, and the
    error says so.
    """

    def __init__(
        self,
        host: str = "127.0.0.1",
        port: int = 2911,
        user: str = "kiva",
        password: str = "",
        *,
        collection: Optional[str] = None,
        workspace: Optional[str] = None,
        timeout: float = 8.0,
        tls: TLS = None,
        reconnect: bool = True,
        types: str = "plain",
    ) -> None:
        if types not in ("plain", "extended"):
            raise ValueError(f'types is "plain" or "extended", not {types!r}')
        if not _is_local(host) and not password:
            raise KivaError("A password is required to connect to a remote server.")
        if collection is not None and workspace is not None and collection != workspace:
            raise ValueError(f"collection {collection!r} and workspace {workspace!r} name two collections: give one")
        collection = collection if collection is not None else workspace
        if collection is not None and not _WORKSPACE.match(collection):
            raise ValueError(f"not a collection name: {collection!r}")
        self.host, self.port, self.user = host, port, user
        self._password = password
        self.collection = collection
        self.timeout = timeout
        self._tls = tls
        self._reconnect = reconnect
        self._sock: Optional[socket.socket] = None
        self._buffer = b""
        self._lock = threading.RLock()
        self._events: Deque[Change] = collections.deque()
        self._in_transaction = False
        self._lost_transaction = False
        self.server: Optional[str] = None
        self.types = types
        self._open()

    # -- connection -------------------------------------------------------

    def _open(self) -> None:
        where = f"{self.host}:{self.port}"
        try:
            raw = socket.create_connection((self.host, self.port), timeout=self.timeout)
        except socket.timeout as e:
            raise KivaTimeoutError(f"No answer from {where}.") from e
        except OSError as e:
            raise KivaConnectionError(f"Cannot reach {where} ({e}). Is KivaDB started?") from e
        raw.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        sock: socket.socket = raw
        try:
            if self._tls:
                sock = self._wrap(raw, where)
            self._sock, self._buffer = sock, b""
            hello = self._call(_HELLO)[1].decode()
            if not hello.startswith("KivaDB"):
                raise KivaError(f"{where} is not a KivaDB server.")
            self.server = hello.split("\n")[0]
            major = re.match(r"KivaDB (\d+)\.", self.server)
            if self.types == "extended" and (major is None or int(major.group(1)) < 2):
                raise KivaError(f'types="extended" needs KivaDB 2.0.0 or later; {where} is {self.server}.')
            if "auth=required" in hello or self._password or self.user != "kiva":
                status, payload = self._call(_AUTH, self.user.encode() + b"\0" + self._password.encode())
                if status != _OK:
                    raise AuthenticationError(payload.decode().replace("permission denied: ", "", 1))
            if self.collection:
                # `workspace switch` is understood by every server, 1.x included.
                self._exec_now(f"workspace switch {self.collection}")
        except BaseException:
            self._drop()
            sock.close()
            raise

    def _wrap(self, raw: socket.socket, where: str) -> socket.socket:
        tls = self._tls
        fingerprint = tls.get("fingerprint") if isinstance(tls, Mapping) else None
        if isinstance(tls, ssl.SSLContext):
            ctx = tls
        else:
            ctx = ssl.create_default_context()
            if fingerprint:
                # A pinned certificate replaces the authorities: KivaDB's own
                # is signed by nobody but itself.
                ctx.check_hostname = False
                ctx.verify_mode = ssl.CERT_NONE
        try:
            sock = ctx.wrap_socket(raw, server_hostname=self.host.strip("[]"))
        except ssl.SSLError as e:
            raw.close()
            raise KivaConnectionError(f"TLS with {where} failed: {e}") from e
        if fingerprint:
            der = sock.getpeercert(binary_form=True) or b""
            seen = hashlib.sha256(der).hexdigest()
            if not _same_fingerprint(seen, fingerprint):
                sock.close()
                raise KivaConnectionError(
                    f"The certificate of {where} (SHA-256 {seen}) is not the expected one: connection refused."
                )
        return sock

    def _drop(self) -> None:
        sock, self._sock = self._sock, None
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass
        if self._in_transaction:
            self._in_transaction = False
            self._lost_transaction = True

    def _ensure(self) -> None:
        if self._sock is not None:
            return
        if self._lost_transaction:
            self._lost_transaction = False
            raise KivaConnectionError(
                "The connection was lost during a transaction: it was rolled back by the server. Start it again."
            )
        if not self._reconnect:
            raise KivaConnectionError("Not connected.")
        self._open()

    @property
    def connected(self) -> bool:
        return self._sock is not None

    def close(self) -> None:
        """Closes the connection. An open transaction is rolled back by the server."""
        with self._lock:
            self._reconnect = False
            self._in_transaction = False
            self._drop()

    def __enter__(self) -> "Client":
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    # -- frames -----------------------------------------------------------

    def _call(self, code: int, payload: bytes = b"") -> Tuple[int, bytes]:
        """Sends a frame and reads its answer; notifications on the way are kept."""
        sock = self._sock
        if sock is None:
            raise KivaConnectionError("Not connected.")
        try:
            sock.settimeout(self.timeout)
            sock.sendall(_HEAD.pack(len(payload) + 2, code) + payload)
            while True:
                status, body = self._read_frame(sock)
                if status == _EVENT:
                    self._events.append(Change(body.decode()))
                    continue
                if status == _BYE:
                    raise KivaConnectionError(body.decode() or "The server closed the connection.")
                return status, body
        except socket.timeout as e:
            self._drop()
            raise KivaTimeoutError(
                f"No answer from {self.host}:{self.port} within {self.timeout} s: the connection is closed, "
                "and whether the request was applied is not known."
            ) from e
        except OSError as e:
            self._drop()
            raise KivaConnectionError(f"The connection to {self.host}:{self.port} was lost ({e}).") from e
        except KivaConnectionError:
            self._drop()
            raise

    def _read_frame(self, sock: socket.socket) -> Tuple[int, bytes]:
        while True:
            if len(self._buffer) >= 6:
                length, status = _HEAD.unpack_from(self._buffer)
                if length < 2 or length > _MAX_FRAME:
                    raise KivaConnectionError(f"a frame of {length} bytes: not KivaDB's protocol")
                if len(self._buffer) >= 4 + length:
                    body = self._buffer[6 : 4 + length]
                    self._buffer = self._buffer[4 + length :]
                    return status, body
            chunk = sock.recv(65536)
            if not chunk:
                raise KivaConnectionError("The connection to KivaDB was closed.")
            self._buffer += chunk

    def _request(self, code: int, payload: bytes = b"") -> Tuple[int, bytes]:
        with self._lock:
            self._ensure()
            return self._call(code, payload)

    # -- requests ---------------------------------------------------------

    def _exec_now(self, command: str) -> List[str]:
        status, payload = self._call(_EXEC, command.encode())
        lines = [line for line in payload.decode().split("\n") if line]
        if status == _ERROR:
            raise KivaError("\n".join(line[1:] for line in lines if line.startswith("-")))
        return [line[1:] for line in lines]

    def exec(self, command: str) -> List[str]:
        """Runs a command of the KivaDB shell language; returns its output lines."""
        with self._lock:
            self._ensure()
            return self._exec_now(command)

    def api(self, request: Mapping[str, Any]) -> Dict[str, Any]:
        """Sends a structured request (``{"op": ...}``); returns the JSON answer."""
        status, payload = self._request(_API, json.dumps(request).encode())
        text = payload.decode()
        if status != _OK:
            raise KivaError(text.replace("permission denied: ", "", 1))
        answer: Dict[str, Any] = json.loads(text)
        return answer

    def _api(self, request: Mapping[str, Any]) -> Dict[str, Any]:
        """A request the driver builds: with ``types="extended"``, it asks for
        extended JSON and reads native values."""
        if self.types == "plain":
            return self.api(request)
        status, payload = self._request(_API, json.dumps({**request, "types": "extended"}).encode())
        text = payload.decode()
        if status != _OK:
            raise KivaError(text.replace("permission denied: ", "", 1))
        answer: Dict[str, Any] = loads_extended(text)
        return answer

    def ping(self) -> None:
        self._request(_PING)

    def get(self, key: str, default: Any = None) -> Any:
        """The value of a key, typed as in :mod:`kivadb.values`, or ``default``."""
        r = self._api({"op": "get", "key": key})
        if not r["found"]:
            return default
        return from_wire(r["type"], r["value"])

    def set(
        self, key: str, value: Any, *, ttl: Optional[int] = None, mode: str = "upsert", type: Optional[str] = None
    ) -> None:
        """Creates or replaces a key. ``mode``: ``upsert``, ``create`` (fails
        if it exists) or ``update`` (fails if it does not). ``ttl`` in seconds;
        ``type`` forces one (``"date"`` for ``"2026-10-27"``, ``"time"``...)."""
        extended = self.types == "extended"
        kiva_type = type or type_of(value, extended)
        req: Dict[str, Any] = {"op": "put", "key": key, "type": kiva_type, "value": to_wire(kiva_type, value, extended)}
        if mode != "upsert":
            req["mode"] = mode
        if ttl is not None:
            req["ttl"] = ttl
        self._api(req)

    def add(self, prefix: str, value: Any, *, ttl: Optional[int] = None, type: Optional[str] = None) -> Tuple[str, str]:
        """Stores a value under a key KivaDB names (``prefix`` and a new
        identifier sorted by time); returns ``(key, id)``."""
        extended = self.types == "extended"
        kiva_type = type or type_of(value, extended)
        req: Dict[str, Any] = {
            "op": "add",
            "prefix": prefix,
            "type": kiva_type,
            "value": to_wire(kiva_type, value, extended),
        }
        if ttl:
            req["ttl"] = ttl
        r = self._api(req)
        return r["key"], r["id"]

    def delete(self, key: str) -> bool:
        """Deletes a key; returns whether it existed."""
        deleted: bool = self.api({"op": "del", "key": key})["deleted"]
        return deleted

    def scan(self, pattern: str = "*", limit: int = 500) -> List[Dict[str, Any]]:
        """Keys matching a pattern, with their type, size and TTL."""
        keys: List[Dict[str, Any]] = self.api({"op": "scan", "pattern": pattern, "limit": limit})["keys"]
        return keys

    @staticmethod
    def _query(
        pattern: str,
        where: Any,
        limit: Optional[int],
        order_by: Optional[str],
        after: Optional[str],
        reverse: bool,
    ) -> Dict[str, Any]:
        req: Dict[str, Any] = {"target": pattern}
        if where is not None:
            req["where"] = condition(where)
        if limit is not None:
            req["limit"] = limit
        if order_by:
            req["order_by"] = order_by
        if after is not None:
            req["after"] = after
        if reverse:
            req["reverse"] = True
        return req

    def query(
        self,
        pattern: str,
        where: Any = None,
        *,
        limit: int = 100,
        order_by: Optional[str] = None,
        after: Optional[str] = None,
        reverse: bool = False,
    ) -> List[Tuple[str, Any]]:
        """The documents of a key pattern that satisfy a condition, as
        ``(key, document)`` pairs. ``where`` is the JSON condition:
        ``{"role": "dev"}``, ``{"field": "age", "cmp": ">", "value": 18}``,
        ``{"and": [...]}``, ``{"or": [...]}``, ``{"not": {...}}``; a moment
        as ``{"$datetime": "2026-10-27T13:30:00Z"}``. ``order_by``:
        ``"created_at desc"``, or ``"random()"`` for ``limit`` documents drawn
        at random, in a random order (KivaDB 2.0.0-rc.4 or later); with
        ``"random()"``, ``where`` may be ``None``: every document of the
        pattern. ``after``: the last key of the previous page.
        ``limit`` at most 10 000."""
        req = self._query(pattern, where, limit, order_by, after, reverse)
        rows = self._api({"op": "query", **req})["rows"]
        return [(r["key"], r["value"]) for r in rows]

    def explain(
        self,
        pattern: str,
        where: Any = None,
        *,
        analyze: bool = False,
        limit: Optional[int] = None,
        order_by: Optional[str] = None,
        after: Optional[str] = None,
        reverse: bool = False,
    ) -> Dict[str, Any]:
        """The plan of a query; with ``analyze``, the query is run and what
        it read is under ``"analyze"``."""
        req = self._query(pattern, where, limit, order_by, after, reverse)
        if analyze:
            req["analyze"] = True
        return self._api({"op": "explain", **req})

    def aggregate(
        self,
        pattern: str,
        compute: Any = "count",
        *,
        where: Any = None,
        group_by: Optional[str] = None,
    ) -> Dict[str, Any]:
        """``count``, ``sum``, ``avg``, ``min``, ``max``:
        ``compute={"n": "count", "mean": {"avg": "age"}}``."""
        req: Dict[str, Any] = {"op": "aggregate", "target": pattern, "compute": compute}
        if where is not None:
            req["where"] = condition(where)
        if group_by:
            req["group_by"] = group_by
        return self._api(req)

    def similar(
        self,
        target: str,
        vector: Sequence[float],
        *,
        limit: int = 10,
        metric: str = "cosine",
        ef: Optional[int] = None,
        exact: bool = False,
    ) -> List[Dict[str, Any]]:
        """Vector search: ``target`` is ``"<pattern>.<field>"``."""
        req: Dict[str, Any] = {"op": "similar", "target": target, "vector": list(vector), "limit": limit, "metric": metric}
        if ef:
            req["ef"] = ef
        if exact:
            req["exact"] = True
        results: List[Dict[str, Any]] = self.api(req)["results"]
        return results

    def changes(
        self,
        cursor: Optional[str] = None,
        *,
        consumer: Optional[str] = None,
        limit: int = 100,
        pattern: Optional[str] = None,
        wait: float = 0,
    ) -> Dict[str, Any]:
        """The change journal (``changes enable``): the changes after
        ``cursor`` (``"beginning"``, ``"now"`` or a cursor), or after a named
        consumer's last acknowledgement. ``wait`` (seconds, at most 30) waits
        for a change when there is none yet. Needs KivaDB 2.0.0."""
        req: Dict[str, Any] = {"op": "changes", "limit": limit}
        if cursor is not None:
            req["cursor"] = cursor
        if consumer is not None:
            req["consumer"] = consumer
        if pattern is not None:
            req["pattern"] = pattern
        if wait:
            req["wait"] = int(wait * 1000)
        if wait and self.timeout < wait + 5:
            with self._lock:
                saved, self.timeout = self.timeout, wait + 5
                try:
                    return self._api(req)
                finally:
                    self.timeout = saved
        return self._api(req)

    def stats(self) -> Dict[str, Any]:
        return self.api({"op": "stats"})

    # -- collections ------------------------------------------------------

    @property
    def workspace(self) -> Optional[str]:
        """The collection, under its former name."""
        return self.collection

    @workspace.setter
    def workspace(self, name: Optional[str]) -> None:
        self.collection = name

    def use(self, collection: str) -> None:
        """Works in another collection from now on, and after a reconnection."""
        if not _WORKSPACE.match(collection):
            raise ValueError(f"not a collection name: {collection!r}")
        if self._in_transaction:
            raise KivaError("cannot change collection inside a transaction")
        self.exec(f"workspace switch {collection}")
        self.collection = collection

    def collections(self, project: Optional[str] = None) -> List[Dict[str, Any]]:
        """The collections the user can reach, those of ``project`` only if
        given: ``name``, ``project``, ``path``, ``access``, ``current``. A
        server older than 2.0.0-rc.5 is asked under the former name."""
        req: Dict[str, Any] = {"op": "collections"}
        if project is not None:
            req["project"] = project
        try:
            found: List[Dict[str, Any]] = self.api(req)["collections"]
        except KivaError as e:
            if "unknown operation" not in str(e):
                raise
            found = self.api({"op": "workspaces"})["workspaces"]
            found = [c for c in found if project is None or c.get("project") == project]
        return found

    def whoami(self) -> Dict[str, Any]:
        """The session: ``user``, ``role``, ``collection`` (``workspace``
        before 2.0.0-rc.5), ``project``, ``access``, ``version``..."""
        return self.api({"op": "whoami"})

    # -- transactions -----------------------------------------------------

    @contextmanager
    def transaction(self) -> Iterator["Client"]:
        """``with db.transaction():`` the writes of the block are committed
        together at its end, or rolled back if it raises. Unique indexes and
        schemas are checked at the commit."""
        with self._lock:
            if self._in_transaction:
                raise KivaError("A transaction is already open on this connection.")
            self.exec("begin")
            self._in_transaction = True
            try:
                yield self
            except BaseException as e:
                if self._lost_transaction:
                    self._lost_transaction = False
                    raise KivaConnectionError(
                        "The connection was lost during the transaction: it was rolled back by the server."
                    ) from e
                if self._in_transaction and self._sock is not None:
                    self._in_transaction = False
                    try:
                        self._exec_now("rollback")
                    except KivaError:
                        pass
                raise
            if not self._in_transaction:
                self._lost_transaction = False
                raise KivaConnectionError(
                    "The connection was lost during the transaction: it was rolled back by the server."
                )
            self._in_transaction = False
            self._exec_now("commit")

    # -- notifications ----------------------------------------------------

    def subscribe(self, pattern: str = "*") -> None:
        """Receives a :class:`Change` for each write to keys matching the
        pattern; read them with :meth:`next_change`."""
        self.exec('subscribe "' + pattern.replace('"', "") + '"')

    def next_change(self, timeout: Optional[float] = None) -> Optional[Change]:
        """The next notification, waiting up to ``timeout`` seconds (the
        client's timeout by default); ``None`` if none came."""
        with self._lock:
            if self._events:
                return self._events.popleft()
            self._ensure()
            sock = self._sock
            assert sock is not None
            deadline = time.monotonic() + (self.timeout if timeout is None else timeout)
            while not self._events:
                left = deadline - time.monotonic()
                if left <= 0:
                    return None
                try:
                    sock.settimeout(left)
                    status, body = self._read_frame(sock)
                except socket.timeout:
                    return None
                except OSError as e:
                    self._drop()
                    raise KivaConnectionError(f"The connection to {self.host}:{self.port} was lost ({e}).") from e
                if status == _EVENT:
                    self._events.append(Change(body.decode()))
            return self._events.popleft()


def connect(
    host: str = "127.0.0.1",
    port: int = 2911,
    user: str = "kiva",
    password: str = "",
    *,
    collection: Optional[str] = None,
    workspace: Optional[str] = None,
    timeout: float = 8.0,
    tls: TLS = None,
    reconnect: bool = True,
    types: str = "plain",
) -> Client:
    """Opens a connection and logs in.

    ``tls``: ``True`` (a certificate from an authority), an
    :class:`ssl.SSLContext`, or ``{"fingerprint": "AB:CD:..."}`` (the
    SHA-256 that ``kivadb tls`` shows on the server, for its own certificate).
    A remote host always needs a password.

    ``types``: ``"plain"`` (the default, the answers of 1.x) or
    ``"extended"`` (KivaDB 2.0.0 and later): ``Decimal``, ``date``,
    ``time`` and aware ``datetime`` are kept inside documents, and decimals
    come back as ``Decimal`` (:mod:`kivadb.values`).

    ``collection``: the collection to work in (``"travel.places"``);
    ``workspace`` is the same option under its former name.
    """
    return Client(
        host,
        port,
        user,
        password,
        collection=collection,
        workspace=workspace,
        timeout=timeout,
        tls=tls,
        reconnect=reconnect,
        types=types,
    )
