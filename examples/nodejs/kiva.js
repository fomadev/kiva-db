// Minimal KivaDB client for Node.js (binary protocol, no dependency).
const net = require("net");

class KivaClient {
  constructor(host = "127.0.0.1", port = 2911) {
    this.socket = net.connect(port, host);
    this.buffer = Buffer.alloc(0);
    this.waiting = [];            // one resolver per request, in order
    this.onEvent = () => {};      // called for change notifications
    this.socket.on("data", (chunk) => this._read(chunk));
    this.socket.on("error", (err) => this.waiting.splice(0).forEach((w) => w.reject(err)));
  }

  // A frame is: [length u32][code u16][payload]; length covers code + payload.
  _send(code, payload = Buffer.alloc(0)) {
    const head = Buffer.alloc(6);
    head.writeUInt32BE(payload.length + 2, 0);
    head.writeUInt16BE(code, 4);
    this.socket.write(Buffer.concat([head, payload]));
    return new Promise((resolve, reject) => this.waiting.push({ resolve, reject }));
  }

  _read(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 6) {
      const length = this.buffer.readUInt32BE(0);
      if (this.buffer.length < 4 + length) return;
      const status = this.buffer.readUInt16BE(4);
      const payload = this.buffer.subarray(6, 4 + length);
      this.buffer = this.buffer.subarray(4 + length);
      if (status === 9) this.onEvent(payload.toString());
      else this.waiting.shift().resolve({ status, payload });
    }
  }

  async auth(user, password) {
    const r = await this._send(2, Buffer.from(`${user}\0${password}`));
    if (r.status !== 0) throw new Error(r.payload.toString());
  }

  // Runs any shell command; returns its output lines.
  async exec(command) {
    const r = await this._send(3, Buffer.from(command));
    const lines = r.payload.toString().split("\n").filter(Boolean).map((l) => l.slice(1));
    if (r.status === 1) throw new Error(lines.join("\n"));
    return lines;
  }

  close() {
    this.socket.end();
  }
}

module.exports = { KivaClient };
