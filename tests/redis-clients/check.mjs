// Redis client libraries, used the way applications commonly use Redis
// (pending registrations with an expiry, resend locks, login rate limits,
// notifying every instance through publish/subscribe),
// against a running KivaDB.
//
//   npm install && node check.mjs <redis port> <password of kiva>
import Redis from "ioredis";
import { createClient } from "redis";
import { RateLimiterRedis } from "rate-limiter-flexible";
import { RedisStore } from "rate-limit-redis";
import assert from "node:assert/strict";

const port = Number(process.argv[2]);
if (!port || process.argv[3] === undefined) {
  console.error("usage: node check.mjs <redis port> <password of kiva>");
  process.exit(2);
}
const url = (db) => `redis://kiva:${process.argv[3]}@127.0.0.1:${port}/${db}`;
let passed = 0;
const step = async (name, fn) => {
  await fn();
  passed++;
  console.log("ok  ", name);
};

// --- Application with ioredis (database 0) ------------------------------
const io = new Redis(url(0));
await step("ioredis: pre-registration stored for 15 minutes", async () => {
  const data = JSON.stringify({ name: "Alice", email: "a@x.cd", hash: "$argon2id$v=19$m=65536", code: "482913" });
  assert.equal(await io.set("pre_reg:a@x.cd", data, "EX", 900), "OK");
  assert.equal(await io.get("pre_reg:a@x.cd"), data);
  assert.ok((await io.ttl("pre_reg:a@x.cd")) > 895);
});
await step("ioredis: resend lock of 60 s refuses a second code", async () => {
  assert.equal(await io.set("resend_lock:a@x.cd", "1", "EX", 60, "NX"), "OK");
  assert.equal(await io.set("resend_lock:a@x.cd", "1", "EX", 60, "NX"), null);
});
await step("ioredis: confirmed registration removes the entry", async () => {
  assert.equal(await io.del("pre_reg:a@x.cd"), 1);
  assert.equal(await io.get("pre_reg:a@x.cd"), null);
});
await step("ioredis: unconfirmed registration disappears by itself", async () => {
  await io.set("pre_reg:late@x.cd", "{}", "PX", 1200);
  // KivaDB keeps expiry times to the second: gone at most 1 s after the requested time.
  await new Promise((r) => setTimeout(r, 1200 + 1100));
  assert.equal(await io.get("pre_reg:late@x.cd"), null);
});
await step("ioredis: attempt counters with INCR + EXPIRE NX, window does not slide", async () => {
  for (let i = 1; i <= 5; i++) {
    const n = await io.incr("rl:login:ip:1.2.3.4");
    assert.equal(n, i);
    await io.expire("rl:login:ip:1.2.3.4", 900, "NX");
  }
  await io.expire("rl:login:ip:1.2.3.4", 5, "NX");
  assert.ok((await io.ttl("rl:login:ip:1.2.3.4")) > 890);
});
await step("ioredis: MULTI pipeline", async () => {
  const r = await io.multi().incr("rl:2fa:acc:42").expire("rl:2fa:acc:42", 900).ttl("rl:2fa:acc:42").exec();
  assert.deepEqual(r.map(([e, v]) => [e, v]), [[null, 1], [null, 1], [null, 900]]);
});
await step("ioredis: concurrent attempts are all counted", async () => {
  const clients = Array.from({ length: 6 }, () => new Redis(url(0)));
  await Promise.all(clients.flatMap((c) => Array.from({ length: 50 }, () => c.incr("rl:burst"))));
  assert.equal(await io.get("rl:burst"), "300");
  clients.forEach((c) => c.disconnect());
});
await step("ioredis: defineCommand (EVALSHA with EVAL fallback)", async () => {
  io.defineCommand("hit", {
    numberOfKeys: 1,
    lua: "local n = redis.call('INCR', KEYS[1]) if n == 1 then redis.call('EXPIRE', KEYS[1], ARGV[1]) end return n",
  });
  assert.equal(await io.hit("rl:custom", 900), 1);
  assert.equal(await io.hit("rl:custom", 900), 2);
});

// --- rate-limiter-flexible (Lua scripts, EVALSHA) ---------------------------
await step("rate-limiter-flexible: 3 tries allowed, the 4th refused", async () => {
  const limiter = new RateLimiterRedis({ storeClient: io, keyPrefix: "rlflex", points: 3, duration: 900 });
  for (let i = 0; i < 3; i++) await limiter.consume("1.2.3.4");
  await assert.rejects(limiter.consume("1.2.3.4"), (e) => e.remainingPoints === 0 && e.msBeforeNext > 890_000);
  await limiter.delete("1.2.3.4");
  await limiter.consume("1.2.3.4");
});

// --- rate-limit-redis, the store of express-rate-limit (SCRIPT LOAD + EVALSHA)
await step("rate-limit-redis: increment, get, decrement, reset", async () => {
  const store = new RedisStore({ sendCommand: (...args) => io.call(...args), prefix: "erl:" });
  store.init({ windowMs: 15 * 60 * 1000 });
  const a = await store.increment("1.2.3.4");
  const b = await store.increment("1.2.3.4");
  assert.equal(a.totalHits, 1);
  assert.equal(b.totalHits, 2);
  assert.ok(b.resetTime > new Date());
  await store.decrement("1.2.3.4");
  assert.equal((await store.get("1.2.3.4")).totalHits, 1);
  await store.resetKey("1.2.3.4");
  assert.equal(await io.exists("erl:1.2.3.4"), 0);
});

// --- Second application with node-redis on database 2 --------------------------------
const pay = createClient({ url: url(2) });
pay.on("error", (e) => console.error("node-redis:", e.message));
await pay.connect();
await step("node-redis on /2: isolated from database 0", async () => {
  await pay.set("pay:session", "s1", { EX: 900 });
  assert.equal(await pay.get("pay:session"), "s1");
  assert.equal(await io.get("pay:session"), null);
  assert.equal(await pay.get("rl:burst"), null);
});
await step("node-redis: SET NX EX, INCR, EXPIRE NX, TTL", async () => {
  assert.equal(await pay.set("lock", "1", { NX: true, EX: 60 }), "OK");
  assert.equal(await pay.set("lock", "1", { NX: true, EX: 60 }), null);
  assert.equal(await pay.incr("tries"), 1);
  assert.equal(await pay.expire("tries", 900, "NX"), 1);
  assert.equal(await pay.expire("tries", 10, "NX"), 0);
  assert.ok((await pay.ttl("tries")) > 890);
});
await step("node-redis (RESP3): hashes come back as objects", async () => {
  await pay.hSet("pay:user:1", { name: "Alice", city: "Kinshasa" });
  assert.deepEqual({ ...(await pay.hGetAll("pay:user:1")) }, { name: "Alice", city: "Kinshasa" });
});
await step("node-redis: EVAL", async () => {
  assert.equal(await pay.eval("return redis.call('INCR', KEYS[1])", { keys: ["tries"] }), 2);
});

// --- Publish/subscribe: every instance hears that the texts changed ------------
const nextMessage = (subscribe) =>
  new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("no message within 5 s")), 5000);
    subscribe((channel, message) => {
      clearTimeout(timer);
      resolve([channel, message]);
    });
  });
await step("ioredis: SUBSCRIBE on a dedicated connection, PUBLISH from another", async () => {
  const listener = new Redis(url(2));
  assert.equal(await listener.subscribe("lp:texts:changed"), 1);
  const got = nextMessage((cb) => listener.once("message", cb));
  assert.equal(await io.publish("lp:texts:changed", "fr"), 1);
  assert.deepEqual(await got, ["lp:texts:changed", "fr"]);
  const pat = new Redis(url(0));
  await pat.psubscribe("lp:texts:*");
  const pgot = nextMessage((cb) => pat.once("pmessage", (_p, ch, m) => cb(ch, m)));
  assert.equal(await io.publish("lp:texts:ln", "v2"), 1);
  assert.deepEqual(await pgot, ["lp:texts:ln", "v2"]);
  listener.disconnect();
  pat.disconnect();
});
await step("node-redis (RESP3): subscriber made with duplicate()", async () => {
  const listener = pay.duplicate();
  listener.on("error", (e) => console.error("node-redis listener:", e.message));
  await listener.connect();
  const got = nextMessage((cb) => listener.subscribe("lp:texts:changed", (message, channel) => cb(channel, message)));
  // Wait for the subscription to be confirmed before publishing.
  for (let i = 0; i < 50 && (await pay.pubSubNumSub("lp:texts:changed"))["lp:texts:changed"] < 1; i++) {
    await new Promise((r) => setTimeout(r, 20));
  }
  assert.equal(await pay.publish("lp:texts:changed", "all"), 1);
  assert.deepEqual(await got, ["lp:texts:changed", "all"]);
  await listener.unsubscribe("lp:texts:changed");
  await listener.quit();
});

await pay.quit();
io.disconnect();
console.log(`\n${passed} checks passed with ioredis, node-redis, rate-limiter-flexible and rate-limit-redis.`);
