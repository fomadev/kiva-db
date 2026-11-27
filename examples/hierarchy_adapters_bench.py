#!/usr/bin/env python3
"""Linux loopback benchmark for the three managed adapters (stdlib only)."""

import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
import http.client
import json
import os
from pathlib import Path
import socket
import ssl
import struct
import subprocess
import tempfile
import threading
import time

SELECT = b"bench.app.keys"
PATH = "/v2/workspaces/bench/projects/app/collections/keys"
PASSWORD = "local-measurement-only"
AUTH = "Basic " + base64.b64encode(f"kiva:{PASSWORD}".encode()).decode()
QUERY = b'{"op":"query","target":"doc:*.city","cmp":"=","value":"Paris","limit":10}'


def tcp_request(sock, reader, code, payload):
    sock.sendall(struct.pack("!IH", len(payload) + 2, code) + payload)
    size, status = struct.unpack("!IH", reader.read(6))
    body = reader.read(size - 2)
    if status or len(body) != size - 2:
        raise RuntimeError((status, body))
    return body


def resp_encode(*args):
    return f"*{len(args)}\r\n".encode() + b"".join(
        f"${len(arg)}\r\n".encode() + arg + b"\r\n" for arg in args
    )


def resp_read(reader):
    line = reader.readline()
    if not line or line[:1] == b"-":
        raise RuntimeError(line)
    if line[:1] == b"$":
        size = int(line[1:])
        return None if size < 0 else reader.read(size + 2)[:-2]
    if line[:1] == b"*":
        return [resp_read(reader) for _ in range(int(line[1:]))]
    return line[1:-2]


def workflow(protocol, port, worker, count, context, ready, mode):
    name = f"key{worker}"
    key = name.encode()
    values = json.dumps({"type": "string", "value": "benchmark"}).encode()
    body = json.dumps({"operations": [{"op": "put", "key": name,
                                      "type": "string", "value": "benchmark"}]}).encode()
    auth_start = time.perf_counter_ns()
    if protocol == "rest":
        conn = (http.client.HTTPSConnection("127.0.0.1", port, context=context, timeout=60)
                if context else http.client.HTTPConnection("127.0.0.1", port, timeout=60))

        def request(method, path, data=None):
            conn.request(method, PATH + path, data, {"Authorization": AUTH, "Content-Type": "application/json"})
            response = conn.getresponse()
            payload = response.read()
            if response.status != 200:
                raise RuntimeError((response.status, payload))
            return payload

        put = lambda: request("PUT", f"/keys/{name}", values)
        get = lambda: request("GET", f"/keys/{name}")
        txn = lambda: request("POST", "/batch", body)
        query = lambda: request("POST", "/api", QUERY)
        close = conn.close
        put()
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
            call(2, f"kiva\0{PASSWORD}".encode())
            call(3, b"use collection " + SELECT)

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

            call(b"AUTH", b"kiva", PASSWORD.encode())
            call(b"KIVA.USE", SELECT)
            put = lambda: call(b"SET", key, b"benchmark")
            get = lambda: call(b"GET", key)
            query = lambda: call(b"KIVA.API", QUERY)

            def txn():
                sock.sendall(
                    resp_encode(b"MULTI") + resp_encode(b"SET", key, b"benchmark") + resp_encode(b"EXEC")
                )
                return [resp_read(reader) for _ in range(3)]

            def pipeline():
                sock.sendall(resp_encode(b"GET", key) * 8)
                return [resp_read(reader) for _ in range(8)]

        auth = time.perf_counter_ns() - auth_start
        put()
    if protocol == "rest":
        auth = time.perf_counter_ns() - auth_start

    action = {"get": get, "set": put, "txn": txn, "query": query, "pipeline": pipeline if protocol == "resp" else get}[mode]
    try:
        ready.wait(timeout=120)
        started = time.perf_counter()
        latencies = []
        for _ in range(count):
            before = time.perf_counter_ns()
            action()
            latencies.append(time.perf_counter_ns() - before)
        return started, time.perf_counter(), latencies, auth
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
    return sorted(data)[round((len(data) - 1) * percent)] / 1_000_000


def run(binary, home, args):
    env = dict(os.environ, KIVADB_NO_UPDATE_CHECK="1")
    prefix = [binary, "--home", str(home)]
    bootstrap = [
        f"modify user kiva password {PASSWORD}", "config set auth_required true",
        "hierarchy data", "workspace create bench", "switch bench", "project create app",
        "use app", "collection create keys", "use collection bench.app.keys",
        'set doc:1 {"city":"Paris"}', "create index on doc:* (city)",
    ]
    for login, commands in [([], bootstrap[:2]), (["-u", "kiva", "-p", PASSWORD], bootstrap[2:])]:
        out = subprocess.run(prefix + ["--no-listen"] + login +
                             [arg for command in commands for arg in ("-e", command)],
                             env=env, capture_output=True, text=True, check=True)
        if "[ERR]" in out.stdout + out.stderr:
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
            print("protocol,mode,tls,clients,ops_s,p50_ms,p95_ms,p99_ms,setup_p95_ms,cpu_s,active_rss_kib,active_fds", flush=True)
            for protocol, port in zip(["tcp", "rest", "resp"], ports):
                for n in args.clients:
                    for mode in args.modes:
                        if mode == "pipeline" and protocol != "resp":
                            continue
                        ready = threading.Barrier(n, action=lambda: active.extend(resources(server.pid)))
                        active = []
                        cpu = resources(server.pid)[0]
                        with ThreadPoolExecutor(max_workers=n) as pool:
                            completed = [pool.submit(workflow, protocol, port, i, args.requests, context, ready, mode)
                                         for i in range(n)]
                            data = [result.result() for result in completed]
                        elapsed = max(row[1] for row in data) - min(row[0] for row in data)
                        times = [value for row in data for value in row[2]]
                        factor = 8 if mode == "pipeline" else 1
                        stats = [len(times) * factor / elapsed] + [percentile(times, p) for p in (0.5, 0.95, 0.99)]
                        stats += [percentile([row[3] for row in data], 0.95), resources(server.pid)[0] - cpu]
                        print(f"{protocol},{mode},{args.tls},{n}," + ",".join(f"{x:.3f}" for x in stats)
                              + f",{active[1]},{active[2]}", flush=True)
                        time.sleep(0.15)
        finally:
            server.terminate()
            server.wait(timeout=30)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/kivadb")
    parser.add_argument("--clients", type=int, nargs="+", default=[1, 8, 32, 128])
    parser.add_argument("--requests", type=int, default=16)
    parser.add_argument("--modes", nargs="+", choices=["get", "set", "txn", "pipeline", "query"],
                        default=["get", "set", "txn", "pipeline", "query"])
    parser.add_argument("--tls", action="store_true")
    with tempfile.TemporaryDirectory(prefix="kiva-adapters-bench-") as temp:
        options = parser.parse_args()
        run(str(Path(options.binary).resolve()), Path(temp), options)
