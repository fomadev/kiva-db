# Random values, UUIDs, shuffling and sampling

A design for KivaDB 2.0.0. It covers random numbers, UUIDs, random texts, shuffled and sampled arrays, and documents returned in random order or as a sample.

**Nothing here is implemented.** No code starts before the maintainer approves this note, and each lot waits for its own approval.

Statements about the engine refer to `main` at `e9d6c7b`, which includes C1 and C2, and to the branch of C3. Section 1 lists what was checked.

## 1. Audit: what exists

| Asked | In KivaDB today | Where |
| :--- | :--- | :--- |
| Random functions in commands, documents or queries | **None** | `exec.rs` `time_expr` knows `now()`, `today()`, `current_*` only; `expr.rs` (`print`) knows `now()`, `today()` and the date parts |
| UUID generation | **None** | No crate, no function |
| Array functions (`shuffle`, `sample`) | **None** | Arrays change through `json append`, `json insert` and Redis `JSON.ARR*` only |
| Random order of query results | **None** | `order by <field> [asc\|desc]` (`engine::Order`); rows of one value come in key order |
| Sampling of documents | **None** | `tune vector index ... sample N` takes evenly spaced keys (`step_by`) to tune an index; it is not random and not exposed to queries |
| Scalar functions evaluated inside documents | **None** | A document is a JSON literal (`json_literal`). The typed literal grammar of C4 (TEMPORAL_FORMATS.md 5.2) is where values that are not JSON would be read |
| Values generated at write | Partly | Schema defaults `now()`, `today()` and `current_time`, computed once per write, at the commit for a transaction (`schema::Fill`, `Moment`). Commands accept `now()` as a value (`set date k now()`) |
| Randomness in scripts | Partly | Lua has `math.random` (standard library `MATH`, Lua 5.4's xoshiro256**). It is not cryptographic. A script's writes are replicated as records, so replicas never re-run it |
| Random sources in the engine | Yes, internal | `util::random_bytes` and `util::random_u64` (the `getrandom` crate, the operating system's generator) give salts, file ids, journal ids and staging names; `crypt.rs` uses `ring::rand::SystemRandom` for keys and nonces |

**Finding.** `util::random_bytes` falls back to bytes derived from the clock when the operating system's generator fails, without saying so. Those bytes are predictable. They serve as password salts (`auth.rs`) and would serve as tokens if this note reused the function.

That failure is very unlikely on supported systems, but a salt or a token must never silently become predictable. Lot R0 makes the function report the failure instead. This is a fix, independent of the new functions.

**Done in R0.** One source remains, `util::fill_random` (`getrandom`), with no fallback; `crypt.rs` draws from it too. When it fails, the operation fails with `ERR_RANDOM_UNAVAILABLE`, and nothing is written in place of the random bytes. Each caller:

| Caller | What it draws | On failure |
| :--- | :--- | :--- |
| `auth::User::new`, `set_password` | 16-byte salts | no account; the password, salt and hash stay as they were |
| `crypt::generate_key` (`kivadb keygen`) | master keys | no key, exit code 1 |
| `crypt::Keys::seal` | 96-bit nonces of records, the change journal, hints and vector graphs | nothing encrypted: the write is refused; graphs are not saved, never saved in clear |
| `util::new_id` (`add`) | the process part and counter start of identifiers | no key made |
| `engine` (open, `clear`, compaction), `changes`, `backup::check` | file ids, journal ids, staging names | the operation fails before writing |
| `repl::load_identity` | a new node id | automatic failover does not start, with the reason in its event |
| `failover` | the delay between election attempts | no delay added, never one from the clock |
| `doctor` | a probe file name | reported as a failure |
| `HRANDFIELD`, `SRANDMEMBER`, `SPOP`, `ZRANDMEMBER` | a member's index | an error reply |

The semantic cache's consistency token starts from the clock. It is a sequence, not a secret: it only has to differ from the tokens before a restart.

## 2. Principles

1. **Values, not code.** A function is evaluated by the engine where the grammar has a value, never by reading a text as code. `"uuid()"` in quotes stays a text.
2. **Evaluated once, then stored.** Each occurrence of a function in a command is evaluated once, when the command is read, and the result is written like any literal. Neither the record, the change journal, replication nor a backup holds a function. A replica receives the values the master wrote, and never draws its own.
3. **One source, the operating system's.** Every function draws from the operating system's cryptographic generator (`getrandom`), even `random()`. Then no function is weak by mistake, and the cost stays small at the volumes of a command. When the generator fails, the command fails; nothing falls back to the clock.
4. **No bias.** Integers are drawn by rejection, never by `%`. Shuffling is Fisher–Yates, with each index drawn by rejection.
5. **The native grammar, not SQL.** The functions go where KivaDB already accepts `now()`. No `INSERT`, `SELECT` or `UPDATE` is added.

## 3. The functions

| Function | Result | Rule |
| :--- | :--- | :--- |
| `random()` (alias `rand()`) | `float` in [0, 1) | 53 random bits divided by 2^53: every multiple of 2^-53 in [0, 1), all equally likely |
| `random_int(min, max)` | `int` in [min, max], bounds included | `min` ≤ `max`, both within `i64`. Rejection sampling over the span, which may reach 2^64 |
| `uuid()` | text, UUID version 4 (RFC 9562), lowercase with hyphens | 122 random bits |
| `uuid_v7()` | text, UUID version 7 (RFC 9562) | 48 bits of Unix milliseconds, 74 random bits. Within one process, the ids are strictly increasing: in the same millisecond, or if the clock goes back, the previous timestamp is kept and a 12-bit counter in `rand_a` (RFC 9562 method 1) increases from a random start |
| `random_string(n)` | text of `n` characters from `A-Z a-z 0-9` (62 symbols) | 1 ≤ `n` ≤ 1024. Each character drawn by rejection: about 5.95 bits each, so 16 characters ≈ 95 bits. `random_string(n, "hex")` and `"base64url"` are worth considering for tokens |
| `shuffle(array)` | the same elements in a uniformly random order | Fisher–Yates. The elements keep their types (C3). Up to the document size bound |
| `sample(array, n)` | `n` distinct positions of the array, in random order | Partial Fisher–Yates. `n` greater than the length is an error, not a silent shorter answer |

The texts of `uuid()` and `uuid_v7()` are plain `string` values: 2.0 has no `uuid` type. A `uuid` type, as 16 bytes in KDOC, could come later; it is not proposed here.

`uuid_v7()` ids sort by time as texts. Whether that helps insertion depends on how KivaDB stores keys:

- KivaDB appends every write to its log, so the order of keys does not change the cost of writing;
- the in-memory B-tree of keys and the B-tree indexes would see their inserts near the end.

The gain is not claimed until measured.

## 4. Where they can be written

| Place | Example | Lot | Notes |
| :--- | :--- | :--- | :--- |
| A whole value | `set token random_string(32)`, `set id uuid_v7()`, `set n random_int(1, 6)` | R1 | Next to `now()` in `time_expr`; the value has the function's type |
| `print` | `print uuid()`, `print random_int(1, 100)` | R1 | `expr.rs` already calls `now()` |
| Schema defaults | `{"id": {"type": "string", "default": "uuid_v7()"}}` | R1 | Like `now()`: one value per document, drawn at the write, at the commit for a transaction; the text is a function only for its type, as `now()` is today |
| Lua | `kiva.uuid()`, `kiva.uuid_v7()`, `kiva.random_int(1, 6)`, `kiva.random_string(16)` | R1 | Next to `math.random`, which stays as it is and is documented as not cryptographic |
| Inside documents | `set q:1 {"id": uuid_v7(), "score": random_int(0, 100), "questions": shuffle(["A", "B", "C"])}` | R3 | **Needs the typed literal parser of C4** (TEMPORAL_FORMATS.md 5.4: `serde_json` first, the typed parser only when it fails). The same parser reads `decimal "1.5"`, `date "..."` and these functions. A JSON document stays JSON: a function is read only outside quotes |
| `json set`, `json append` | `json set k .token random_string(16)`, `json set k .order shuffle(.order)`? | R3 | A value position, so the same as above; `shuffle(.path)` of a field already in the document is attractive but is a second feature, left for later |
| Structured requests and REST | `{"op": "put", "key": "k", "value": {"$fn": "uuid_v7"}}`? | open | Programs can generate ids themselves, and drivers do; a JSON form is a new syntax. Not proposed for 2.0.0 unless asked |

## 5. Random order and samples of documents

### 5.1 `order by random()`

`get where <condition> order by random() [limit n]`: every matching document gets a random key, and the rows come in the order of those keys.

| Case | Cost |
| :--- | :--- |
| With `limit n` | every matching document is read. The existing `TopK` heap keeps `n` rows: memory for `n` rows, time O(N log n) |
| Without `limit` | **refused**: every matching document would be held and sorted in memory. Today `order by <field>` without a limit does exactly that, without a bound; a random order is not worth that risk, and `sample` gives a random selection without it |

It is never cheap on a large pattern. The manual says so, with the advice to use `sample` instead.

`after <key>` is refused with `order by random()`, since a random order has no next page. An index cannot help.

### 5.2 `sample n`

`get where <condition> sample n` gives `n` documents chosen uniformly among those the query selects, in random order, or all of them when fewer match.

| What decides the condition | How |
| :--- | :--- |
| Indexes only (every condition answered by an index), or no condition | Reservoir sampling (Vitter's Algorithm L) over the **keys** the indexes or the prefix give, then only the `n` documents chosen are read. Uniform over the matching keys, for O(n) memory and few reads |
| A condition that needs the document | Each candidate is read and tested, and the reservoir keeps `n` of those that pass: uniform over the matching documents; every candidate is read, as for any such query |

`explain` says which applies. A sample is drawn from what the reader can see: grants apply per workspace in KivaDB, so every document of the workspace counts.

Inside a transaction, the query sees what any query sees there: the committed data and the transaction's own writes.

`sample` takes no `after` and no `order by`. Two runs give two samples. A `seed` for reproducible samples would need a deterministic generator, kept apart from principle 3; it is left open.

### 5.3 Exposure

| Path | `order by random()` / `sample n` |
| :--- | :--- |
| Shell, binary protocol | as above |
| REST and structured requests | `"order_by": "random()"`, `"sample": 5` |
| Redis | `KIVA.API` with the structured request; no Redis command has this meaning |
| Node.js, Python | through their structured queries, one option each (lot R4) |

## 6. Transactions, replication, journal, backups, indexes

| Question | Decision |
| :--- | :--- |
| When a function is evaluated | When the command is read, once per occurrence. A schema default is drawn at the write, at the commit for a transaction, like `now()` |
| Retries | A command is never retried by KivaDB. A program that resends a command after an error gets new values, as it would get a new `now()`; drivers do not resend writes (the Python driver documents it) |
| Replication | Records carry values. A replica draws nothing, and replicas therefore never disagree |
| Change journal | It holds the values written |
| Backups and restores | Files are copied; nothing is drawn again |
| Indexes | KivaDB has no index on an expression, so no index can hold a random expression. A value drawn by a function is indexed like any other |
| Unique indexes | A drawn value that collides is refused like any duplicate. `random_int` collides often by nature; `uuid()` practically never (122 bits) |
| `uuid_v7()` across nodes | Increasing within one process only. Two nodes, or a restart within the same millisecond, may interleave; uniqueness rests on the 74 random bits, not on the order |

## 7. Limits and errors

| Case | Answer |
| :--- | :--- |
| `random_int(5, 1)`, or a bound that is not an integer | refused, naming the bounds |
| `random_string(0)` or `random_string(5000)` | refused: 1 to 1024 |
| `sample([1, 2], 3)` | refused: 3 positions asked of an array of 2 |
| `shuffle` or `sample` of something that is not an array | refused, naming its type |
| The operating system's generator fails | the command fails with an error; nothing is written |
| `order by random()` without `limit` | refused, with the advice to add a limit or use `sample` |

## 8. Tests planned

- **Unit tests:**
  - every bound of `random_int`, including spans of 1 and of 2^64;
  - the UUID version and variant bits;
  - `uuid_v7` increasing, with a fake clock for the same millisecond and for a clock going back;
  - the alphabet of `random_string`;
  - `shuffle` and `sample` keep the elements and their types;
  - errors.
- **Statistical tests**, with fixed sample sizes and loose thresholds, so that they are not flaky:
  - a chi-square test on `random_int(1, 6)` and on the positions after `shuffle` of 4 elements;
  - the frequency of each document in `sample n` over many runs.

  These show that there is no gross bias. **They do not prove cryptographic quality**, and the documentation will not claim that they do.
- **Replication:** a master writes `uuid_v7()`, `random_int` and a schema default, and the replica holds the same values. The same check for the change journal.
- **Queries:** `sample` reads `n` documents when the indexes decide; `order by random()` with `limit`, and refused without; `after` refused; `explain`.

## 9. Risks

| Risk | Likelihood | Impact | Mitigation |
| :--- | :--- | :--- | :--- |
| A predictable value used as a token | Low with principle 3 | High | One source, the OS generator; no fallback (R0); `math.random` documented as not cryptographic |
| Modulo bias or a biased shuffle | Medium if written quickly | Medium | Rejection sampling; Fisher–Yates; statistical tests |
| A replica drawing other values than its master | Certain if functions were replicated as text | Very high | Principle 2: only values are stored |
| `order by random()` exhausting memory or time | High on large patterns | Medium | A limit is required and uses the heap; `sample` documented as the cheap way |
| A text taken for a function | Low | High | Functions only outside quotes, only in value positions, only names in a fixed list |
| Users expecting `uuid_v7()` to be ordered across nodes | Medium | Low | Documented (section 6) |
| Grammar conflicts with C4 | Medium | Medium | R3 comes after C4 and is built in its parser |
| The scope delays 2.0.0 | Medium | Medium | Independent lots; R3 can move to 2.1 without affecting the rest |

## 10. Work in pull requests

| Lot | Content | Depends on | Size |
| :--- | :--- | :--- | :--- |
| R0 | `util::random_bytes` reports a failure of the OS generator instead of falling back to the clock. A fix, with a test of the error path | — | Small |
| R1 | `random`, `rand`, `random_int`, `uuid`, `uuid_v7` and `random_string` as whole values, in `print`, in schema defaults and in Lua; unit, statistical and replication tests; manual | R0 | Medium |
| R2 | `order by random()` and `sample n`, in the shell, the binary protocol, REST and structured requests; `explain`; tests and measurements | R1 | Medium |
| R3 | The functions inside documents and in `json set`, including `shuffle` and `sample` of arrays | C4, R1 | Medium |
| R4 | The Node.js and Python drivers: the query options of R2 | R2, C6 | Small |

None starts before its approval. R0 can be done at any time.

## 11. Recommendation for 2.0.0

| Function | For 2.0.0 | Why |
| :--- | :--- | :--- |
| `uuid_v7()`, `uuid()` | **Yes** (R1) | Ids made by the server, the most asked; small and well specified |
| `random_string(n)` | **Yes** (R1) | Codes and tokens, with the OS generator |
| `random_int(min, max)`, `random()` | **Yes** (R1) | Small once the generator is there |
| `sample n` in queries | **Yes** (R2) | The useful way to get random documents, cheap when indexes decide |
| `order by random()` | Yes, with its cost documented (R2) | Comes almost free with `TopK`; useful for small patterns |
| `shuffle(array)`, `sample(array, n)` in documents | If C4 is ready in time (R3) | They need the typed literal parser; otherwise 2.1 |
| A `uuid` type, a `seed` for samples, functions in REST bodies | No | Not needed for the uses described; each adds syntax or a type |

Order: R0 first, as it is a fix. Then R1 and R2 after C3 is merged, and R3 after C4. The integrity of documents (C3 to C5) stays ahead of these functions.
