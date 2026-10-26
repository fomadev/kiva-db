// Runs the driver against a real KivaDB server started for the test.
// The executable is taken from KIVADB_BIN, or from ../../target/{debug,release}.
const test = require("node:test");
const assert = require("node:assert");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { connect, KivaError } = require("..");

const exe = process.platform === "win32" ? "kivadb.exe" : "kivadb";
const candidates = [
  process.env.KIVADB_BIN,
  path.join(__dirname, "..", "..", "..", "target", "debug", exe),
  path.join(__dirname, "..", "..", "..", "target", "release", exe),
].filter(Boolean);
const binary = candidates.find((p) => fs.existsSync(p));

function freePort() {
  return new Promise((resolve) => {
    const s = net.createServer().listen(0, "127.0.0.1", () => {
      const { port } = s.address();
      s.close(() => resolve(port));
    });
  });
}

async function waitFor(port) {
  for (let i = 0; i < 100; i++) {
    try {
      await new Promise((resolve, reject) => net.connect(port, "127.0.0.1").once("connect", function () { this.destroy(); resolve(); }).once("error", reject));
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error("KivaDB did not start");
}

// Waits for the server to exit before removing its home: on Windows, its
// files stay open until then, and the removal fails with ENOTEMPTY.
async function stop(server, home) {
  if (server.exitCode === null && server.signalCode === null) {
    await new Promise((resolve) => {
      server.once("exit", resolve);
      server.kill();
    });
  }
  fs.rmSync(home, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
}

test("the driver talks to a real KivaDB", { skip: !binary && "KivaDB is not built (set KIVADB_BIN)" }, async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "kivadb-node-"));
  const port = await freePort();
  const server = spawn(binary, ["serve", "--home", home, "--port", String(port), "--no-http", "--no-resp"], { stdio: "ignore" });
  t.after(() => stop(server, home));
  await waitFor(port);

  await t.test("refuses an empty password for a remote host", async () => {
    await assert.rejects(connect({ host: "203.0.113.7", password: "" }), /password is required/);
  });

  const db = await connect({ port });
  // The driver has its own version; the server reports the one in Cargo.toml.
  const cargo = fs.readFileSync(path.join(__dirname, "..", "..", "..", "Cargo.toml"), "utf8");
  assert.equal(db.server, `KivaDB ${cargo.match(/^version = "(.+)"/m)[1]}`);

  await t.test("typed values round-trip", async () => {
    await db.set("s", "both \" and ' quotes");
    await db.set("n", 3.5);
    await db.set("b", true);
    await db.set("z", null);
    await db.set("list", [1, "two"]);
    await db.set("doc", { name: "Alice", tags: ["a"] }, { ttl: 60 });
    await db.set("raw", Buffer.from([0, 1, 255]));
    assert.equal(await db.get("s"), "both \" and ' quotes");
    assert.equal(await db.get("n"), 3.5);
    assert.equal(await db.get("b"), true);
    assert.equal(await db.get("z"), null);
    assert.deepEqual(await db.get("list"), [1, "two"]);
    assert.deepEqual(await db.get("doc"), { name: "Alice", tags: ["a"] });
    assert.deepEqual(await db.get("raw"), Buffer.from([0, 1, 255]));
    assert.equal(await db.get("ghost"), undefined);
    const keys = await db.scan("d*");
    assert.equal(keys[0].key, "doc");
    assert.ok(keys[0].ttl > 50);
  });

  await t.test("modes, deletes and errors", async () => {
    await assert.rejects(db.set("s", "x", { mode: "create" }), KivaError);
    await assert.rejects(db.set("ghost", "x", { mode: "update" }), /not found/);
    assert.equal(await db.del("s"), true);
    assert.equal(await db.del("s"), false);
    await assert.rejects(db.exec('update n "text"'), /Cannot update \[number\]/);
    assert.deepEqual(await db.exec("get n and ghost"), ["n: 3.5 [number]", "ghost: (nil)"]);
  });

  await t.test("vector search", async () => {
    await db.set("doc:1", { t: "cat", embedding: [1, 0, 0] });
    await db.set("doc:2", { t: "car", embedding: [0, 0, 1] });
    const [best] = await db.similar("doc:*.embedding", [0.9, 0.1, 0], { limit: 1 });
    // Semantic cache: a near-identical question gets the stored answer.
    assert.equal(await db.semanticStore("faq", { text: "Capital?" }, { vector: [1, 0, 0], ttl: 60 }), "faq:1");
    const hit = await db.semanticLookup("faq", { vector: [1, 0.05, 0] });
    assert.deepEqual(hit.answer, { text: "Capital?" });
    assert.ok(hit.similarity > 0.99);
    assert.equal(await db.semanticLookup("faq", { vector: [0, 1, 0] }), null);
    // One call: computed once, then cached; not kept if its data changed meanwhile.
    let calls = 0;
    const model = async () => (calls += 1, "cat");
    const ask = { text: "First doc?", depends: ["doc:*"] };
    assert.equal(await db.semanticCached("rag", ask, model), "cat");
    assert.equal(await db.semanticCached("rag", ask, model), "cat");
    assert.equal(calls, 1);
    const changing = async () => (await db.set("doc:3", { t: "new" }), "stale");
    assert.equal(await db.semanticCached("rag", { text: "Newest doc?", depends: ["doc:*"] }, changing), "stale");
    assert.equal(await db.semanticLookup("rag", { text: "Newest doc?" }), null);
    assert.equal(best.key, "doc:1");
    assert.equal(best.value.t, "cat");
  });

  await t.test("change notifications", async () => {
    const seen = [];
    db.on("change", (e) => seen.push(e));
    await db.subscribe("order:*");
    await db.set("order:1", { total: 5 });
    await db.del("order:1");
    await db.ping();
    for (let i = 0; i < 20 && seen.length < 2; i++) await new Promise((r) => setTimeout(r, 50));
    assert.deepEqual(seen.map((e) => [e.event, e.key, e.type]), [["set", "order:1", "object"], ["del", "order:1", null]]);
  });

  await t.test("many requests in flight come back in order", async () => {
    const out = await Promise.all(Array.from({ length: 40 }, (_, i) => db.exec(`print ${i} * 3`)));
    assert.deepEqual(out.map((o) => o[0]), Array.from({ length: 40 }, (_, i) => String(i * 3)));
  });

  db.close();
  await assert.rejects(db.ping(), /Not connected/);
});

test("the driver encrypts the connection with TLS", { skip: !binary && "KivaDB is not built (set KIVADB_BIN)" }, async (t) => {
  const { execFileSync } = require("node:child_process");
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "kivadb-node-tls-"));
  const port = await freePort();
  const server = spawn(binary, ["serve", "--home", home, "--port", String(port), "--no-http", "--no-resp"], { stdio: "ignore" });
  t.after(() => stop(server, home));
  await waitFor(port);
  const shown = execFileSync(binary, ["--home", home, "tls"]).toString();
  const fingerprint = /SHA-256\s*:\s*(\S+)/.exec(shown)[1];

  const db = await connect({ port, tls: { fingerprint } });
  await db.set("secret", "encrypted");
  assert.strictEqual(await db.get("secret"), "encrypted");
  db.close();

  // Without the fingerprint, the self-signed certificate is not trusted.
  await assert.rejects(connect({ port, tls: true }), KivaError);
  const wrong = "AB:".repeat(31) + "AB";
  await assert.rejects(connect({ port, tls: { fingerprint: wrong } }), /not the expected one/);
});
