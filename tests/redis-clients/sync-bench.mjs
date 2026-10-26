// Durable writes from many Redis clients at once, with storage.sync_writes on.
// node sync-bench.mjs <port> <password> [clients] [seconds]
import Redis from "ioredis";

const [port, password, clients = "16", seconds = "3"] = process.argv.slice(2);
const conns = Array.from({ length: Number(clients) }, () => new Redis({ port: Number(port), password }));
// Log in first: checking a password is slow on purpose.
await Promise.all(conns.map((c) => c.ping()));
const end = Date.now() + Number(seconds) * 1000;
const latencies = [];
await Promise.all(
  conns.map(async (c, n) => {
    for (let i = 0; Date.now() < end; i++) {
      const t = process.hrtime.bigint();
      await c.set(`bench:${n}:${i % 1000}`, "x".repeat(100));
      latencies.push(Number(process.hrtime.bigint() - t) / 1e6);
    }
  }),
);
latencies.sort((a, b) => a - b);
const p = (q) => latencies[Math.floor(latencies.length * q)].toFixed(2);
console.log(
  `${clients} clients: ${Math.round(latencies.length / Number(seconds))} writes/s, p50 ${p(0.5)} ms, p99 ${p(0.99)} ms`,
);
conns.forEach((c) => c.disconnect());
