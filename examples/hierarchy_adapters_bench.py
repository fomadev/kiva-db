#!/usr/bin/env python3
"""Linux loopback benchmark for the three managed adapters (stdlib only)."""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from contextlib import closing
import http.client
import json
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import sys
import tempfile
import threading
import time

SELECT = b"bench.app.keys"
PATH = "/v2/workspaces/bench/projects/app/collections/keys"
PASSWORD = "local-measurement-only"
AUTH = "Basic " + base64.b64encode(f"kiva:{PASSWORD}".encode()).decode()
QUERY = b'{"op":"query","target":"doc:*.city","cmp":"=","value":"Paris","limit":10}'


class Refused(RuntimeError):
    """The server rejected the request before dispatch."""


def tcp_request(sock, reader, code, payload):
    sock.sendall(struct.pack("!IH", len(payload) + 2, code) + payload)
    size, status = struct.unpack("!IH", reader.read(6))
    body = reader.read(size - 2)
    if status and body.startswith(b"ERR_SERVER_BUSY:"):
        raise Refused(body)
    if status or len(body) != size - 2:
        raise RuntimeError((status, body))
    return body


def resp_encode(*args):
    return f"*{len(args)}\r\n".encode() + b"".join(
        f"${len(arg)}\r\n".encode() + arg + b"\r\n" for arg in args
    )


def resp_read(reader):
    line = reader.readline()
    if line.startswith(b"-BUSYERR"):
        raise Refused(line)
    if not line or line[:1] == b"-":
        raise RuntimeError(line)
    if line[:1] == b"$":
        size = int(line[1:])
        return None if size < 0 else reader.read(size + 2)[:-2]
    if line[:1] == b"*":
        return [resp_read(reader) for _ in range(int(line[1:]))]
    return line[1:-2]


def http_connection(port, context):
    if context:
        return http.client.HTTPSConnection("127.0.0.1", port, context=context, timeout=60)
    return http.client.HTTPConnection("127.0.0.1", port, timeout=60)


def http_request(conn, authorization, method, path, data=None, expected=200):
    conn.request(method, path, data, {"Authorization": authorization, "Content-Type": "application/json"})
    response = conn.getresponse()
    payload = response.read()
    if response.status == 503:
        raise Refused(payload)
    if response.status != expected:
        raise RuntimeError(f"unexpected HTTP status {response.status} on {method} {path}")
    return payload


def workflow(protocol, port, worker, args, context, ready, finished, mode):
    collection = ("keys", "extra")[worker % 2]
    account = ("writer1", "writer2")[worker % 2]
    selection = f"bench.app.{collection}".encode()
    collection_path = PATH.removesuffix("keys") + collection
    basic = "Basic " + base64.b64encode(f"{account}:{PASSWORD}".encode()).decode()
    name = f"key{worker}"
    key = name.encode()
    values = json.dumps({"type": "string", "value": "benchmark"}).encode()
    body = json.dumps({"operations": [{"op": "put", "key": name,
                                      "type": "string", "value": "benchmark"}]}).encode()
    auth_start = time.perf_counter_ns()
    if protocol in ("rest", "bearer"):
        conn = http_connection(port, context)
        authorization = basic

        def request(method, path, data=None):
            return http_request(conn, authorization, method, collection_path + path, data=data)

        if protocol == "bearer":
            session = http_request(conn, basic, "POST", "/v2/auth/login", expected=201)
            authorization = "Bearer " + json.loads(session)["access_token"]
        else:
            # Basic establishes the connection with its first ordinary write.
            request("PUT", f"/keys/{name}", values)
        auth = time.perf_counter_ns() - auth_start
        put = lambda: request("PUT", f"/keys/{name}", values)
        get = lambda: request("GET", f"/keys/{name}")
        txn = lambda: request("POST", "/batch", body)
        query = lambda: request("POST", "/api", QUERY)

        def close():
            try:
                if protocol == "bearer":
                    http_request(conn, authorization, "POST", "/v2/auth/logout")
            finally:
                conn.close()
    else:
        sock = socket.create_connection(("127.0.0.1", port), timeout=60)
        sock.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
        if context:
            sock = context.wrap_socket(sock, server_hostname="127.0.0.1")
        reader = sock.makefile("rb")

        def close():
            reader.close()
            sock.close()

        if protocol == "tcp":
            def call(code, payload):
                return tcp_request(sock, reader, code, payload)

            call(9, b"hierarchy/1")
            call(2, f"{account}\0{PASSWORD}".encode())
            call(3, b"use collection " + selection)

            def put():
                return call(8, json.dumps({"op": "put", "key": name, "type": "string", "value": "benchmark"}).encode())

            get = lambda: call(5, key)
            query = lambda: call(8, QUERY)

            def txn():
                call(8, b'{"op":"begin"}')
                put()
                return call(8, b'{"op":"commit"}')
        else:
            def call(*args):
                sock.sendall(resp_encode(*args))
                return resp_read(reader)

            call(b"AUTH", account.encode(), PASSWORD.encode())
            call(b"KIVA.USE", selection)
            put = lambda: call(b"SET", key, b"benchmark")
            get = lambda: call(b"GET", key)
            query = lambda: call(b"KIVA.API", QUERY)

            def txn():
                sock.sendall(
                    resp_encode(b"MULTI") + resp_encode(b"SET", key, b"benchmark") + resp_encode(b"EXEC")
                )
                resp_read(reader)
                resp_read(reader)
                try:
                    return resp_read(reader)
                except Refused:
                    call(b"DISCARD")
                    raise

            def pipeline():
                sock.sendall(resp_encode(b"GET", key) * 8)
                refused = None
                for _ in range(8):
                    try:
                        resp_read(reader)
                    except Refused as err:
                        refused = err
                if refused:
                    raise refused

        auth = time.perf_counter_ns() - auth_start
        put()

    action = {"get": get, "set": put, "txn": txn, "query": query, "pipeline": pipeline if protocol == "resp" else get}[mode]
    try:
        for _ in range(args.warmup):
            action()
        ready.wait(timeout=120)
        started = time.perf_counter()
        latencies = []
        refused = 0
        attempts = 0
        while attempts < args.requests or time.perf_counter() - started < args.seconds:
            attempts += 1
            before = time.perf_counter_ns()
            try:
                action()
            except Refused:
                refused += 1
            else:
                latencies.append(time.perf_counter_ns() - before)
        stopped = time.perf_counter()
        finished.wait(timeout=120)
        return started, stopped, latencies, auth, refused
    finally:
        close()


def free_ports():
    sockets = [socket.socket() for _ in range(3)]
    try:
        for sock in sockets:
            sock.bind(("127.0.0.1", 0))
        return [sock.getsockname()[1] for sock in sockets]
    finally:
        for sock in sockets:
            sock.close()


def resources(pid):
    base = Path(f"/proc/{pid}")
    stat = (base / "stat").read_text().rsplit(") ", 1)[1].split()
    cpu = (int(stat[11]) + int(stat[12])) / os.sysconf("SC_CLK_TCK")
    rss = next(int(line.split()[1]) for line in (base / "status").read_text().splitlines() if line.startswith("VmRSS:"))
    return cpu, rss, len(list((base / "fd").iterdir()))


def percentile(data, percent):
    return sorted(data)[round((len(data) - 1) * percent)] / 1_000_000 if data else float("nan")


def probe_boundaries(ports, context):
    with socket.create_connection(("127.0.0.1", ports[0]), timeout=15) as admin, admin.makefile("rb") as wire:
        tcp_request(admin, wire, 9, b"hierarchy/1")
        tcp_request(admin, wire, 2, f"kiva\0{PASSWORD}".encode())
        redis = socket.create_connection(("127.0.0.1", ports[2]), timeout=15)
        if context:
            redis = context.wrap_socket(redis, server_hostname="127.0.0.1")
        with redis, redis.makefile("rb") as replies:
            for command in [(b"AUTH", b"reader", PASSWORD.encode()), (b"KIVA.USE", SELECT), (b"GET", b"key0")]:
                redis.sendall(resp_encode(*command))
                resp_read(replies)
            before = time.perf_counter_ns()
            tcp_request(admin, wire, 3, b"hierarchy revoke reader read collection " + SELECT)
            revoked = time.perf_counter_ns()
            redis.sendall(resp_encode(b"GET", b"key0"))
            assert replies.readline().startswith(b"-NOPERM ")
            print(f"revoke_ms={(revoked - before) / 1e6:.3f},deny_ms={(time.perf_counter_ns() - revoked) / 1e6:.3f}",
                  file=sys.stderr)

    # Claim each slot by exchanging a frame before opening the next socket.
    # Probe the managed connection cap over loopback without a handshake.
    sockets = []
    try:
        for _ in range(128):
            sock = socket.create_connection(("127.0.0.1", ports[2]), timeout=15)
            sockets.append(sock)
            with sock.makefile("rb") as replies:
                sock.sendall(resp_encode(b"PING"))
                assert resp_read(replies) == b"PONG"
        before = time.perf_counter_ns()
        with socket.create_connection(("127.0.0.1", ports[2]), timeout=15) as sock, sock.makefile("rb") as replies:
            assert replies.readline().startswith(b"-BUSYERR ")
        print(f"connection_cap=128,refusal_ms={(time.perf_counter_ns() - before) / 1e6:.3f}", file=sys.stderr)
    finally:
        for sock in sockets:
            sock.close()


def run(binary, home, args):
    env = dict(os.environ, KIVADB_NO_UPDATE_CHECK="1")
    prefix = [binary, "--home", str(home)]
    accounts = [
        f"modify user kiva password {PASSWORD}",
        f"create user reader identified by {PASSWORD} role viewer",
        f"create user writer1 identified by {PASSWORD} role editor",
        f"create user writer2 identified by {PASSWORD} role editor",
        "config set auth_required true",
    ]
    if args.metrics:
        accounts.append("config set server.native_metrics true")
    records = json.dumps({"city": "London", "payload": "x" * 100})
    setup = ["hierarchy data", "workspace create bench", "switch bench", "project create app", "use app"]
    for collection, account, count in [("keys", "writer1", args.records), ("extra", "writer2", args.records * 4)]:
        setup += [f"collection create {collection}", f"use collection bench.app.{collection}", "begin"]
        setup += [f"set doc:{i} {records}" for i in range(count)]
        setup += ['set doc:Paris {"city":"Paris"}', "commit", "create index on doc:* (city)",
                  f"hierarchy grant {account} write collection bench.app.{collection}",
                  "use app"]
    setup.append("hierarchy grant reader read collection bench.app.keys")
    for login, commands in [([], accounts), (["-u", "kiva", "-p", PASSWORD], setup)]:
        out = subprocess.run(prefix + ["--no-listen"] + login +
                             [arg for command in commands for arg in ("-e", command)],
                             env=env, capture_output=True, text=True)
        if out.returncode or "[ERR]" in out.stdout + out.stderr:
            raise RuntimeError(out.stdout + out.stderr)
    ports = free_ports()
    command = prefix + ["serve", "--hierarchical-tcp", "--hierarchical-http", "--hierarchical-resp"]
    for flag, port in zip(["--port", "--http-port", "--resp-port"], ports):
        command += [flag, str(port)]
    with (home / "benchmark-server.log").open("w+") as log:
        server = subprocess.Popen(command, env=env, stdout=log, stderr=log)
        try:
            for _ in range(400):
                if server.poll() is not None:
                    raise RuntimeError((home / "benchmark-server.log").read_text())
                try:
                    with socket.create_connection(("127.0.0.1", ports[0]), timeout=1):
                        break
                except OSError:
                    time.sleep(0.05)
            else:
                raise RuntimeError("server startup timed out")
            context = ssl.create_default_context(cafile=str(home / "tls/cert.pem")) if args.tls else None
            print("protocol,mode,tls,clients,run,ops_s,p50_ms,p95_ms,p99_ms,setup_p95_ms,cpu_s,active_rss_kib,active_fds,refused", flush=True)
            listeners = dict(zip(("tcp", "rest", "resp"), ports))
            listeners["bearer"] = listeners["rest"]
            for protocol in args.protocols:
                port = listeners[protocol]
                for n in args.clients:
                    for (mode, repeat) in [(mode, repeat) for mode in args.modes for repeat in range(args.repetitions)]:
                        if mode == "pipeline" and protocol != "resp":
                            continue
                        ready = threading.Barrier(n, action=lambda: active.extend(resources(server.pid)))
                        finished = threading.Barrier(n)
                        active = []
                        cpu = resources(server.pid)[0]
                        with ThreadPoolExecutor(max_workers=n) as pool:
                            completed = [pool.submit(workflow, protocol, port, i, args, context, ready, finished, mode)
                                         for i in range(n)]
                            data = [result.result() for result in completed]
                        elapsed = max(row[1] for row in data) - min(row[0] for row in data)
                        times = [value for row in data for value in row[2]]
                        factor = 8 if mode == "pipeline" else 1
                        stats = [len(times) * factor / elapsed] + [percentile(times, p) for p in (0.5, 0.95, 0.99)]
                        stats += [percentile([row[3] for row in data], 0.95), resources(server.pid)[0] - cpu]
                        print(f"{protocol},{mode},{args.tls},{n},{repeat + 1}," + ",".join(f"{x:.3f}" for x in stats)
                              + f",{active[1]},{active[2]},{sum(row[4] for row in data)}", flush=True)
                        time.sleep(0.25)
            if "bearer" in args.protocols:
                with closing(http_connection(ports[1], context)) as conn:
                    session = json.loads(http_request(conn, AUTH, "POST", "/v2/auth/login", expected=201))
                    bearer = "Bearer " + session["access_token"]
                    start = time.perf_counter_ns()
                    http_request(conn, AUTH, "POST", "/v2/auth/revoke", b'{"account":"kiva"}')
                    revoked = time.perf_counter_ns()
                    http_request(conn, bearer, "GET", "/v2/auth/session", expected=401)
                    print(f"bearer_revoke_ms={(revoked - start) / 1e6:.3f},deny_ms={(time.perf_counter_ns() - revoked) / 1e6:.3f}",
                          file=sys.stderr)
                    if args.metrics:
                        print(http_request(conn, AUTH, "GET", "/v2/auth/metrics").decode(), file=sys.stderr)
            probe_boundaries(ports, context)
        finally:
            server.terminate()
            server.wait(timeout=30)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/kivadb")
    parser.add_argument("--clients", type=int, nargs="+", default=[1, 8, 32, 128])
    parser.add_argument("--requests", type=int, default=32, help="Minimum measured attempts per worker")
    parser.add_argument("--seconds", type=float, default=1.0, help="Minimum measured seconds per worker")
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--repetitions", type=int, default=2)
    parser.add_argument("--records", type=int, default=128, help="Seed documents in the first collection, four times as many in the second")
    parser.add_argument("--protocols", nargs="+", choices=["tcp", "rest", "resp", "bearer"],
                        default=["tcp", "rest", "resp", "bearer"])
    parser.add_argument("--metrics", action="store_true", help="Include opt-in aggregate stage timings")
    parser.add_argument("--modes", nargs="+", choices=["get", "set", "txn", "pipeline", "query"],
                        default=["get", "set", "txn", "pipeline", "query"])
    parser.add_argument("--tls", action="store_true")
    with tempfile.TemporaryDirectory(prefix="kiva-adapters-bench-") as temp:
        options = parser.parse_args()
        run(str(Path(options.binary).resolve()), Path(temp), options)
