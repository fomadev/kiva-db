// Native values in extended mode (docs/architecture/NATIVE_DOCUMENTS.md,
// section 23), against a real KivaDB server started for the test.
const test = require("node:test");
const assert = require("node:assert");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");
const { connect, KivaError, _parseExtended: parse } = require("..");

const exe = process.platform === "win32" ? "kivadb.exe" : "kivadb";
const binary = [
  process.env.KIVADB_BIN,
  path.join(__dirname, "..", "..", "..", "target", "debug", exe),
  path.join(__dirname, "..", "..", "..", "target", "release", exe),
]
  .filter(Boolean)
  .find((p) => fs.existsSync(p));

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
      await new Promise((resolve, reject) =>
        net.connect(port, "127.0.0.1").once("connect", function () { this.destroy(); resolve(); }).once("error", reject));
      return;
    } catch {
      await new Promise((r) => setTimeout(r, 100));
    }
  }
  throw new Error("KivaDB did not start");
}

async function stop(server, home) {
  if (server.exitCode === null && server.signalCode === null) {
    await new Promise((resolve) => {
      server.once("exit", resolve);
      server.kill();
    });
  }
  fs.rmSync(home, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 });
}

/** A decimal class of the program's choice: here, one that keeps its text. */
class Dec {
  constructor(text) {
    this.text = String(text);
  }
  toString() {
    return this.text;
  }
}

test("the extended parser reads what JSON.parse reads, integers exact", () => {
  for (const text of ['{"a":[1,-2.5,1e3,true,false,null,"x\\"y\\u00e9\\ud83d\\ude00"]}', "[]", "{}", ' [ 0 , -0 , {"b" : {}} ] ', '"\\n"']) {
    assert.deepStrictEqual(parse(text), JSON.parse(text), text);
  }
  assert.strictEqual(parse("9007199254740991"), 9007199254740991);
  assert.strictEqual(parse("9007199254740992"), 9007199254740992n);
  assert.strictEqual(parse("-9007199254740993"), -9007199254740993n);
  assert.strictEqual(parse("9223372036854775807"), 9223372036854775807n);
  assert.strictEqual(parse("9007199254740993.0"), 9007199254740992);
  assert.strictEqual(Object.getPrototypeOf(parse('{"__proto__": {"x": 1}}')), Object.prototype);
  assert.deepStrictEqual(parse('{"__proto__": 1}')["__proto__"], 1);
  for (const bad of ["", "[1,]", "{a:1}", "01", "[1] x", '"open', "tru"]) {
    assert.throws(() => parse(bad), SyntaxError, bad);
  }
});

test("options are checked before connecting", async () => {
  await assert.rejects(connect({ types: "typed" }), /types is "plain" or "extended"/);
  await assert.rejects(connect({ types: "extended", decimal: "Decimal" }), /decimal is a class/);
});

test("extended mode is refused by a server older than 2.0.0", async () => {
  const fake = net.createServer((s) => {
    s.on("data", () => {
      const payload = Buffer.from("KivaDB 1.7.1\nauth=none\nuser=kiva");
      const head = Buffer.alloc(6);
      head.writeUInt32BE(payload.length + 2, 0);
      head.writeUInt16BE(0, 4);
      s.write(Buffer.concat([head, payload]));
    });
  });
  await new Promise((r) => fake.listen(0, "127.0.0.1", r));
  const { port } = fake.address();
  try {
    await assert.rejects(connect({ port, types: "extended" }), /needs KivaDB 2.0.0 or later; .* is KivaDB 1.7.1/);
    const db = await connect({ port });
    assert.equal(db.server, "KivaDB 1.7.1");
    db.close();
  } finally {
    fake.close();
  }
});

test("native values round-trip in extended mode", { skip: !binary && "KivaDB is not built (set KIVADB_BIN)" }, async (t) => {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "kivadb-node-ext-"));
  const port = await freePort();
  const server = spawn(binary, ["serve", "--home", home, "--port", String(port), "--no-http", "--no-resp"], { stdio: "ignore" });
  t.after(() => stop(server, home));
  await waitFor(port);

  const plain = await connect({ port });
  const ext = await connect({ port, types: "extended" });
  const dec = await connect({ port, types: "extended", decimal: Dec });
  t.after(() => [plain, ext, dec].forEach((c) => c.close()));

  await plain.exec(
    'set acc:1 {"name": "Ana", "balance": decimal "1250.50", "opened": date "2026-01-15", ' +
      '"at": datetime "2026-10-31T15:18:00Z", "alarm": time "08:30:00", "big": 9007199254740993, "rate": float 8}',
  );

  await t.test("reading", async () => {
    const p = await plain.get("acc:1");
    assert.equal(p.balance, "1250.50");
    assert.equal(p.big, 9007199254740992, "plain JSON rounds, as before");
    const e = await ext.get("acc:1");
    assert.equal(e.balance, "1250.50");
    assert.equal(e.opened, "2026-01-15");
    assert.equal(e.alarm, "08:30:00");
    assert.ok(e.at instanceof Date);
    assert.equal(e.at.toISOString(), "2026-10-31T15:18:00.000Z");
    assert.strictEqual(e.big, 9007199254740993n);
    assert.strictEqual(e.rate, 8);
    const d = await dec.get("acc:1");
    assert.ok(d.balance instanceof Dec);
    assert.equal(String(d.balance), "1250.50");
  });

  await t.test("writing", async () => {
    const doc = {
      price: new Dec("9.90"),
      when: new Date("2026-02-02T10:00:00.250Z"),
      big: 2n ** 62n,
      $ref: "kept",
      list: [new Dec("0.10"), 1],
    };
    await dec.set("item:1", doc);
    assert.deepEqual(await plain.exec("json type item:1 .price"), ["item:1.price: [decimal]"]);
    assert.deepEqual(await plain.exec("json type item:1 .when"), ["item:1.when: [datetime]"]);
    const back = await dec.get("item:1");
    assert.equal(String(back.price), "9.90");
    assert.equal(back.when.toISOString(), "2026-02-02T10:00:00.250Z");
    assert.strictEqual(back.big, 2n ** 62n);
    assert.equal(back.$ref, "kept");
    assert.equal(String(back.list[0]), "0.10");
    const raw = await plain.api({ op: "get", key: "item:1" });
    assert.equal(raw.value.$ref, "kept", "a $ key of the data is stored as it is");

    // Keys of their kinds.
    await dec.set("d", new Dec("0.10"));
    await ext.set("i", 2n ** 60n);
    await ext.set("at", new Date("2026-01-01T00:00:00Z"));
    assert.deepEqual(await plain.exec("typeof d and i and at"), [" -> d is a [decimal]", " -> i is a [int]", " -> at is a [datetime]"]);
    assert.equal(String(await dec.get("d")), "0.10");
    assert.strictEqual(await ext.get("i"), 2n ** 60n);
    await assert.rejects(ext.set("huge", 2n ** 64n), /beyond the 64-bit integers/);

    // A decimal needs extended mode.
    const plainDec = await connect({ port, decimal: Dec });
    await assert.rejects(plainDec.set("d", new Dec("1")), /types: "extended"/);
    plainDec.close();

    // Plain mode is unchanged: a Date inside a document is its text.
    await plain.set("p", { when: new Date("2026-01-01T00:00:00Z") });
    assert.deepEqual(await plain.exec("json type p .when"), ["p.when: [string]"]);
  });

  await t.test("conditions carry native values in both modes", async () => {
    await plain.exec("create index on acc:*.balance");
    const decPlain = await connect({ port, decimal: Dec });
    for (const c of [decPlain, dec]) {
      const rows = await c.query("acc:*", { field: "balance", cmp: "=", value: new Dec("1250.5") });
      assert.deepEqual(rows.map((r) => r.key), ["acc:1"]);
      const at = await c.query("acc:*", { field: "at", cmp: ">=", value: new Date("2026-10-31T15:00:00Z") });
      assert.deepEqual(at.map((r) => r.key), ["acc:1"]);
    }
    decPlain.close();
    const rows = await dec.query("acc:*", { name: "Ana" });
    assert.ok(rows[0].value.balance instanceof Dec);
  });
});
