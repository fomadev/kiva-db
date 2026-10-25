import { EventEmitter } from "events";

export interface ConnectOptions {
  /** Default "127.0.0.1". */
  host?: string;
  /** Default 2911. */
  port?: number;
  /** Default "kiva", the built-in administrator. */
  user?: string;
  /** Required for any address other than this computer. */
  password?: string;
  /** Milliseconds; default 8000. */
  timeout?: number;
  /**
   * Encrypt the connection. `true` checks the certificate against the usual
   * authorities; `{ fingerprint }` accepts only the certificate with that
   * SHA-256 fingerprint (shown by `kivadb tls` on the server); other keys are
   * passed to Node's `tls.connect` (for example `ca`).
   */
  tls?: boolean | ({ fingerprint?: string } & Record<string, unknown>);
}

export type KivaType = "string" | "number" | "boolean" | "null" | "array" | "object" | "bytes";

export interface SetOptions {
  /** Seconds until the key expires; 0 removes an expiry. Omitted keeps the current one. */
  ttl?: number;
  /** "upsert" (default), "create" (fails if the key exists) or "update" (fails if it does not). */
  mode?: "upsert" | "create" | "update";
  /** Forces the KivaDB type instead of deriving it from the JavaScript value. */
  type?: KivaType;
}

export interface KeyInfo {
  key: string;
  type: KivaType;
  size: number;
  modified: number;
  ttl: number | null;
  compression: "none" | "lz4" | "zlib";
  msgpack: boolean;
}

export interface SimilarOptions {
  /** Default 10. */
  limit?: number;
  /** Default "cosine". */
  metric?: "cosine" | "dot" | "l2";
  /** HNSW index: candidates kept by the search (default 100); higher is more accurate and slower. */
  ef?: number;
  /** Compare every vector even when an HNSW index exists. */
  exact?: boolean;
}

export interface SimilarResult<T = unknown> {
  key: string;
  /** Cosine and dot: higher is closer. L2: lower is closer. */
  score: number;
  value: T;
}

export interface ChangeEvent {
  event: "set" | "update" | "del" | "reset";
  key: string | null;
  type: KivaType | null;
  raw: string;
}

export class KivaError extends Error {}

export class KivaClient extends EventEmitter {
  /** First line of the server greeting, e.g. "KivaDB 1.0.0". */
  server: string;
  static connect(options?: ConnectOptions): Promise<KivaClient>;
  /** Runs any command of the KivaDB shell language; rejects with KivaError on an error line. */
  exec(command: string): Promise<string[]>;
  /** Structured request: whoami, workspaces, scan, get, put, del, similar, stats, users. */
  api<T = any>(request: { op: string; [field: string]: unknown }): Promise<T>;
  ping(): Promise<void>;
  get<T = unknown>(key: string): Promise<T | undefined>;
  set(key: string, value: unknown, options?: SetOptions): Promise<void>;
  del(key: string): Promise<boolean>;
  scan(pattern?: string, limit?: number): Promise<KeyInfo[]>;
  similar<T = unknown>(target: string, vector: number[], options?: SimilarOptions): Promise<SimilarResult<T>[]>;
  /** Semantic cache: the answer stored for this question or a near-identical one, or null. */
  semanticLookup<T = unknown>(
    cache: string,
    question: { text?: string; vector?: number[]; threshold?: number },
  ): Promise<{ answer: T; similarity: number; id: string } | null>;
  /** Semantic cache: remembers an answer; returns its id, or null when data it depends on changed after `since`. */
  semanticStore(
    cache: string,
    answer: unknown,
    question: { text?: string; vector?: number[]; ttl?: number; depends?: string[]; since?: number },
  ): Promise<string | null>;
  /** Semantic cache in one call: the cached answer, or compute()'s, stored unless its data changed meanwhile. */
  semanticCached<T = unknown>(
    cache: string,
    question: { text?: string; vector?: number[]; threshold?: number; ttl?: number; depends?: string[] },
    compute: () => Promise<T>,
  ): Promise<T>;
  stats(): Promise<Record<string, unknown>>;
  /** After this, "change" events are emitted for matching keys. */
  subscribe(pattern?: string): Promise<void>;
  close(): void;
  on(event: "change", listener: (e: ChangeEvent) => void): this;
  on(event: "close", listener: () => void): this;
  on(event: string, listener: (...args: any[]) => void): this;
}

export function connect(options?: ConnectOptions): Promise<KivaClient>;
