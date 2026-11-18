"use strict";
// Official Node.js driver for KivaDB (binary protocol, port 2911).
// No dependencies: only Node's own `net`, `tls` and `events` modules.

const net = require("net");
const tls = require("tls");
const { EventEmitter } = require("events");

const CMD = { HELLO: 1, AUTH: 2, EXEC: 3, PING: 4, API: 8 };
const ST = { OK: 0, ERROR: 1, NOT_FOUND: 2, BYE: 3, EVENT: 9 };

class KivaError extends Error {
  constructor(message) {
    super(message);
    this.name = "KivaError";
    // "UNIQUE: ...", "READONLY: ..." (after "Error: " for exec): the word before
    // the colon, for code to test.
    const m = /^(?:Error: )?([A-Z][A-Z_]+): /.exec(message);
    this.code = m ? m[1] : undefined;
  }
}

// Extended JSON (docs/architecture/NATIVE_DOCUMENTS.md, section 23): asked
// for with connect({ types: "extended" }), it carries every type of KivaDB
// without loss. Plain JSON, the default, is what 1.x drivers always read.

const I64_MIN = -(2n ** 63n);
const I64_MAX = 2n ** 63n - 1n;

function setOwn(o, k, v) {
  if (k === "__proto__") Object.defineProperty(o, k, { value: v, enumerable: true, writable: true, configurable: true });
  else o[k] = v;
}

/**
 * JSON.parse that keeps integers exact: one beyond ±(2^53 - 1) becomes a
 * BigInt. (The reviver of JSON.parse sees the source text only from
 * Node.js 21, so the driver parses extended answers itself.)
 */
function parseExtended(text) {
  let i = 0;
  const fail = () => {
    throw new SyntaxError(`Unexpected token in JSON at position ${i}`);
  };
  const ws = () => {
    while (i < text.length && (text[i] === " " || text[i] === "\t" || text[i] === "\n" || text[i] === "\r")) i++;
  };
  const string = () => {
    const start = i++;
    while (i < text.length && text[i] !== '"') i += text[i] === "\\" ? 2 : 1;
    if (i >= text.length) fail();
    i++;
    return JSON.parse(text.slice(start, i));
  };
  const NUMBER = /-?(?:0|[1-9]\d*)(\.\d+)?([eE][+-]?\d+)?/y;
  const number = () => {
    NUMBER.lastIndex = i;
    const m = NUMBER.exec(text);
    if (!m) fail();
    i = NUMBER.lastIndex;
    if (m[1] === undefined && m[2] === undefined) {
      const n = Number(m[0]);
      if (!Number.isSafeInteger(n)) return BigInt(m[0]);
      return n;
    }
    return Number(m[0]);
  };
  const value = () => {
    ws();
    const c = text[i];
    if (c === "{") {
      i++;
      const o = {};
      ws();
      if (text[i] === "}") return i++, o;
      for (;;) {
        ws();
        if (text[i] !== '"') fail();
        const k = string();
        ws();
        if (text[i++] !== ":") fail();
        setOwn(o, k, value());
        ws();
        const d = text[i++];
        if (d === "}") return o;
        if (d !== ",") fail();
      }
    }
    if (c === "[") {
      i++;
      const a = [];
      ws();
      if (text[i] === "]") return i++, a;
      for (;;) {
        a.push(value());
        ws();
        const d = text[i++];
        if (d === "]") return a;
        if (d !== ",") fail();
      }
    }
    if (c === '"') return string();
    for (const [word, v] of [["true", true], ["false", false], ["null", null]]) {
      if (text.startsWith(word, i)) return (i += word.length), v;
    }
    return number();
  };
  const v = value();
  ws();
  if (i !== text.length) fail();
  return v;
}

/** A value read in extended JSON as JavaScript: tags become their values, `$$` keys `$` keys. */
function fromExtended(v, decimal) {
  if (Array.isArray(v)) return v.map((x) => fromExtended(x, decimal));
  if (v === null || typeof v !== "object") return v;
  const keys = Object.keys(v);
  if (keys.length === 1 && typeof v[keys[0]] === "string") {
    const text = v[keys[0]];
    switch (keys[0]) {
      case "$decimal":
        return decimal ? new decimal(text) : text;
      case "$date":
      case "$time":
        return text;
      case "$datetime":
        return new Date(text);
    }
  }
  const o = {};
  for (const k of keys) setOwn(o, k.startsWith("$$") ? k.slice(1) : k, fromExtended(v[k], decimal));
  return o;
}

// How a part of a request is written: a document (`value`), a condition
// (`where`), or the rest of the request.
const PART = { DOC: 1, COND: 2, OTHER: 3 };

/** JSON of a request, with BigInt, Date and decimals written as KivaDB reads them. */
function writeJson(v, part, ctx) {
  if (v === null || v === undefined) return "null";
  switch (typeof v) {
    case "bigint":
      if (v < I64_MIN || v > I64_MAX) throw new KivaError(`${v} is beyond the 64-bit integers of KivaDB`);
      return v.toString();
    case "number":
    case "boolean":
    case "string":
      return JSON.stringify(v);
    case "function":
    case "symbol":
      return "null";
  }
  const tags = part === PART.COND || (part === PART.DOC && ctx.extended);
  if (v instanceof Date) return tags ? JSON.stringify({ $datetime: v.toISOString() }) : JSON.stringify(v);
  if (ctx.decimal && v instanceof ctx.decimal) {
    return tags ? JSON.stringify({ $decimal: String(v) }) : JSON.stringify(String(v));
  }
  if (typeof v.toJSON === "function") return writeJson(v.toJSON(), part, ctx);
  if (Array.isArray(v)) return "[" + v.map((x) => writeJson(x, part, ctx)).join(",") + "]";
  const escape = part === PART.DOC && ctx.extended;
  const out = [];
  for (const [k, x] of Object.entries(v)) {
    if (x === undefined || typeof x === "function" || typeof x === "symbol") continue;
    const key = escape && k.startsWith("$") ? "$" + k : k;
    out.push(JSON.stringify(key) + ":" + writeJson(x, part, ctx));
  }
  return "{" + out.join(",") + "}";
}

/** A structured request built by the driver, as JSON. */
function requestJson(request, ctx) {
  const parts = [];
  for (const [k, v] of Object.entries(request)) {
    if (v === undefined) continue;
    const part = k === "value" ? PART.DOC : k === "where" ? PART.COND : PART.OTHER;
    parts.push(JSON.stringify(k) + ":" + writeJson(v, part, ctx));
  }
  return "{" + parts.join(",") + "}";
}

/** The major version in a HELLO answer ("KivaDB 2.0.0-rc.2"). */
function serverMajor(hello) {
  const m = /^KivaDB (\d+)\./.exec(hello);
  return m ? Number(m[1]) : 0;
}

/** "AB:CD:..." or "abcd...": the same fingerprint, compared without case or colons. */
function sameFingerprint(a, b) {
  const norm = (s) => String(s || "").replace(/^(sha256\/|SHA256:)/, "").replace(/:/g, "").toLowerCase();
  return norm(a).length === 64 && norm(a) === norm(b);
}

function isLocalHost(host) {
  const h = String(host).trim().toLowerCase();
  return h === "localhost" || h === "::1" || h === "[::1]" || h.startsWith("127.");
}

/**
 * A connection to a KivaDB server.
 *
 * Events: "change" (event object), "close", "error".
 */
class KivaClient extends EventEmitter {
  constructor(socket) {
    super();
    this._socket = socket;
    this._buffer = Buffer.alloc(0);
    this._waiting = [];
    this._closed = false;
    this._ctx = { extended: false, decimal: undefined };
    socket.on("data", (chunk) => this._onData(chunk));
    socket.on("error", (err) => this._fail(err));
    socket.on("close", () => this._fail(new KivaError("The connection to KivaDB was closed.")));
  }

  /**
   * Opens a connection and logs in.
   * @param {object} [options] host (localhost), port (2911), user ("kiva"), password (""), timeout (8000 ms),
   *   tls: true, or { fingerprint } (the server's certificate, from `kivadb tls`), or Node TLS options such as { ca },
   *   types: "plain" (default) or "extended" (native values without loss; needs KivaDB 2.0.0),
   *   decimal: a class built from a text, e.g. Decimal of decimal.js, for the decimals read in extended mode
   */
  static connect(options = {}) {
    const types = options.types === undefined ? "plain" : options.types;
    if (types !== "plain" && types !== "extended") {
      return Promise.reject(new KivaError(`types is "plain" or "extended", not ${JSON.stringify(types)}.`));
    }
    if (options.decimal !== undefined && typeof options.decimal !== "function") {
      return Promise.reject(new KivaError("decimal is a class built from a text, such as Decimal of decimal.js."));
    }
    const host = options.host || "127.0.0.1";
    const port = options.port || 2911;
    const user = options.user || "kiva";
    const password = options.password || "";
    const timeout = options.timeout || 8000;
    if (!isLocalHost(host) && !password) {
      return Promise.reject(new KivaError("A password is required to connect to a remote server."));
    }
    const secure = options.tls ? (options.tls === true ? {} : options.tls) : null;
    const fingerprint = secure && secure.fingerprint;
    return new Promise((resolve, reject) => {
      let socket;
      if (secure) {
        const { fingerprint: _, ...nodeOptions } = secure;
        socket = tls.connect({
          host,
          port,
          servername: net.isIP(host) ? undefined : host,
          // A pinned fingerprint replaces the usual authority check: KivaDB's
          // own certificate is signed by nobody but itself.
          ...(fingerprint ? { rejectUnauthorized: false } : {}),
          ...nodeOptions,
        });
      } else {
        socket = net.connect({ host, port });
      }
      socket.setNoDelay(true);
      const timer = setTimeout(() => {
        socket.destroy();
        reject(new KivaError(`No answer from ${host}:${port}.`));
      }, timeout);
      socket.once("error", (err) => {
        clearTimeout(timer);
        reject(new KivaError(`Cannot reach ${host}:${port} (${err.message}). Is KivaDB started?`));
      });
      socket.once(secure ? "secureConnect" : "connect", async () => {
        clearTimeout(timer);
        if (fingerprint) {
          const seen = socket.getPeerCertificate().fingerprint256;
          if (!sameFingerprint(seen, fingerprint)) {
            socket.destroy();
            reject(new KivaError(`The certificate of ${host}:${port} (${seen}) is not the expected one: connection refused.`));
            return;
          }
        }
        const client = new KivaClient(socket);
        try {
          const hello = (await client._call(CMD.HELLO)).payload.toString();
          if (!hello.startsWith("KivaDB")) throw new KivaError(`${host}:${port} is not a KivaDB server.`);
          client.server = hello.split("\n")[0];
          if (types === "extended" && serverMajor(client.server) < 2) {
            throw new KivaError(`types: "extended" needs KivaDB 2.0.0 or later; ${host}:${port} is ${client.server}.`);
          }
          client._ctx = { extended: types === "extended", decimal: options.decimal };
          if (hello.includes("auth=required") || password || user !== "kiva") {
            const r = await client._call(CMD.AUTH, Buffer.concat([Buffer.from(user), Buffer.from([0]), Buffer.from(password)]));
            if (r.status !== ST.OK) throw new KivaError(r.payload.toString().replace(/^permission denied: /, ""));
          }
          resolve(client);
        } catch (err) {
          client.close();
          reject(err);
        }
      });
    });
  }

  // A frame is [length u32][code u16][payload], big-endian; length covers code + payload.
  _call(code, payload = Buffer.alloc(0)) {
    if (this._closed) return Promise.reject(new KivaError("Not connected."));
    const head = Buffer.alloc(6);
    head.writeUInt32BE(payload.length + 2, 0);
    head.writeUInt16BE(code, 4);
    this._socket.write(Buffer.concat([head, payload]));
    return new Promise((resolve, reject) => this._waiting.push({ resolve, reject }));
  }

  _onData(chunk) {
    this._buffer = this._buffer.length ? Buffer.concat([this._buffer, chunk]) : chunk;
    while (this._buffer.length >= 6) {
      const length = this._buffer.readUInt32BE(0);
      if (this._buffer.length < 4 + length) return;
      const status = this._buffer.readUInt16BE(4);
      const payload = this._buffer.subarray(6, 4 + length);
      this._buffer = this._buffer.subarray(4 + length);
      if (status === ST.EVENT) {
        this.emit("change", parseEvent(payload.toString()));
      } else {
        const waiter = this._waiting.shift();
        if (waiter) waiter.resolve({ status, payload });
      }
    }
  }

  _fail(err) {
    if (this._closed) return;
    this._closed = true;
    for (const w of this._waiting.splice(0)) w.reject(err instanceof KivaError ? err : new KivaError(err.message));
    this.emit("close");
  }

  /** Runs any command of the KivaDB shell language and returns its output lines. */
  async exec(command) {
    const r = await this._call(CMD.EXEC, Buffer.from(command));
    const lines = r.payload.toString().split("\n").filter(Boolean);
    const output = lines.map((l) => l.slice(1));
    if (r.status === ST.ERROR) {
      throw new KivaError(lines.filter((l) => l.startsWith("-")).map((l) => l.slice(1)).join("\n"));
    }
    return output;
  }

  /**
   * A request the driver builds: in extended mode it asks for extended JSON
   * and reads native values; conditions carry native values in both modes.
   */
  async _api(request) {
    const ctx = this._ctx;
    const req = ctx.extended ? { ...request, types: "extended" } : request;
    const r = await this._call(CMD.API, Buffer.from(requestJson(req, ctx)));
    const text = r.payload.toString();
    if (r.status !== ST.OK) throw new KivaError(text.replace(/^permission denied: /, ""));
    return ctx.extended ? parseExtended(text) : JSON.parse(text);
  }

  /** A value of an answer, as JavaScript. */
  _value(v) {
    return this._ctx.extended ? fromExtended(v, this._ctx.decimal) : v;
  }

  /** Sends a structured request ({op: ...}) as written and returns the JSON answer. */
  async api(request) {
    const r = await this._call(CMD.API, Buffer.from(JSON.stringify(request)));
    const text = r.payload.toString();
    if (r.status !== ST.OK) throw new KivaError(text.replace(/^permission denied: /, ""));
    return JSON.parse(text);
  }

  async ping() {
    await this._call(CMD.PING);
  }

  /**
   * The value of a key, or undefined. Bytes come back as a Buffer, a datetime
   * as a Date; a date ("2026-10-27") and a time ("14:30:00") as text.
   */
  async get(key) {
    const r = await this._api({ op: "get", key });
    if (!r.found) return undefined;
    return fromWire(r.type, this._value(r.value));
  }

  /**
   * Creates or replaces a key. The type follows the JavaScript value.
   * @param {object} [options] ttl (seconds), mode ("upsert" | "create" | "update"), type (to force one)
   */
  async set(key, value, options = {}) {
    const type = options.type || this._typeOf(value);
    const req = { op: "put", key, type, value: toWire(type, value) };
    if (options.mode) req.mode = options.mode;
    if (options.ttl !== undefined) req.ttl = options.ttl;
    await this._api(req);
  }

  /**
   * Stores a value under a key KivaDB names: the prefix followed by a new
   * identifier (24 hexadecimal characters, ObjectId format, sorted by time).
   * Needs KivaDB 1.7.0 or later.
   * @param {string} prefix e.g. "place:"
   * @param {object} [options] ttl (seconds), type (to force one)
   * @returns {Promise<{key: string, id: string}>}
   */
  async add(prefix, value, options = {}) {
    const type = options.type || this._typeOf(value);
    const req = { op: "add", prefix, type, value: toWire(type, value) };
    if (options.ttl) req.ttl = options.ttl;
    const r = await this._api(req);
    return { key: r.key, id: r.id };
  }

  /**
   * The documents of a key pattern that satisfy a condition, in key order.
   * The condition is JSON: {type: "VILLE"} (equalities), {city: ["Goma", "Bukavu"]}
   * (in), {field: "name", cmp: "ilike", value: "kin%"}, {and: [...]}, {or: [...]},
   * {not: {...}}. in, like and ilike need KivaDB 1.7.0 or later.
   * @param {string} pattern e.g. "place:*"
   * @param {object} where the condition
   * @param {object} [options] limit (100, at most 10000)
   * @returns {Promise<{key: string, value: any}[]>}
   */
  async query(pattern, where, options = {}) {
    const r = await this._api({ op: "query", target: pattern, where, limit: options.limit || 100 });
    return r.rows.map((row) => ({ key: row.key, value: this._value(row.value) }));
  }

  /** Deletes a key; resolves to whether it existed. */
  async del(key) {
    return (await this.api({ op: "del", key })).deleted;
  }

  /** Keys with their metadata. */
  async scan(pattern = "*", limit = 500) {
    return (await this.api({ op: "scan", pattern, limit })).keys;
  }

  /**
   * Vector similarity search.
   * @param {string} target "<pattern>.<field>", e.g. "doc:*.embedding"
   * @param {number[]} vector the query vector
   * @param {object} [options] limit (10), metric ("cosine" | "dot" | "l2"),
   *   ef (candidates of an HNSW search, default 100), exact (compare every vector)
   */
  async similar(target, vector, options = {}) {
    const req = { op: "similar", target, vector, limit: options.limit || 10, metric: options.metric || "cosine" };
    if (options.ef) req.ef = options.ef;
    if (options.exact) req.exact = true;
    const r = await this.api(req);
    return r.results;
  }

  async stats() {
    return this.api({ op: "stats" });
  }

  /**
   * Semantic cache: the answer stored for this question or a near-identical
   * one, or null. Give the question's embedding (vector), its text, or both.
   * @param {string} cache cache name
   * @param {object} question text, vector, threshold (default 0.98)
   */
  async semanticLookup(cache, question = {}) {
    const r = await this.api({ op: "semantic", action: "lookup", cache, ...question });
    return r.hit ? { answer: r.answer, similarity: r.similarity, id: r.id } : null;
  }

  /**
   * Semantic cache: remembers the answer to a question. Returns its id, or
   * null when `since` was given and data it depends on changed after it.
   * @param {object} question text, vector, ttl (seconds), depends (key patterns), since
   */
  async semanticStore(cache, answer, question = {}) {
    const r = await this.api({ op: "semantic", action: "store", cache, answer, ...question });
    return r.id;
  }

  /**
   * Semantic cache in one call: the cached answer, or `compute()`'s, which
   * is then stored, unless data it depends on changed while it was computed.
   * @param {object} question text, vector, threshold, ttl, depends
   * @param {() => Promise<unknown>} compute calls the model
   */
  async semanticCached(cache, question, compute) {
    const { ttl, depends, ...ask } = question;
    const r = await this.api({ op: "semantic", action: "lookup", cache, ...ask });
    if (r.hit) return r.answer;
    const answer = await compute();
    const { threshold, ...q } = ask;
    await this.api({ op: "semantic", action: "store", cache, answer, ...q, ttl, depends, since: r.since });
    return answer;
  }

  /** Receives "change" events for keys matching the pattern. */
  async subscribe(pattern = "*") {
    await this.exec(`subscribe "${String(pattern).replace(/"/g, "")}"`);
  }

  /** The KivaDB type of a JavaScript value; in extended mode, a BigInt is an int and a decimal a decimal. */
  _typeOf(value) {
    const { extended, decimal } = this._ctx;
    if (decimal && value instanceof decimal) {
      if (!extended) throw new KivaError('A decimal is written with connect({ types: "extended" }).');
      return "decimal";
    }
    if (typeof value === "bigint" && extended) return "int";
    return typeOf(value);
  }

  close() {
    this._closed = true;
    this._socket.end();
    this._socket.destroy();
  }
}

// A value as the structured requests carry it: bytes in base64, a datetime in ISO 8601.
function toWire(type, value) {
  if (type === "bytes") return Buffer.from(value).toString("base64");
  if (type === "datetime" && value instanceof Date) return value.toISOString();
  return value;
}

function fromWire(type, value) {
  if (type === "bytes") return Buffer.from(value, "base64");
  if (type === "datetime") return new Date(value);
  return value;
}

function typeOf(value) {
  if (value === null || value === undefined) return "null";
  if (Buffer.isBuffer(value) || value instanceof Uint8Array) return "bytes";
  if (value instanceof Date) return "datetime";
  if (Array.isArray(value)) return "array";
  switch (typeof value) {
    case "string":
      return "string";
    case "number":
    case "bigint":
      return "number";
    case "boolean":
      return "boolean";
    default:
      return "object";
  }
}

function parseEvent(text) {
  // "set user:1 (object)" / "del user:1" / "reset (all keys deleted)"
  const [event, key, type] = text.split(" ");
  return { event, key: event === "reset" ? null : key, type: type ? type.replace(/[()]/g, "") : null, raw: text };
}

module.exports = { KivaClient, KivaError, connect: KivaClient.connect, _parseExtended: parseExtended };
