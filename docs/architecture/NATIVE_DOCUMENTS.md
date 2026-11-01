# Native typed values in documents and storage format 3

The design of option C for KivaDB 2.0.0: documents and arrays that keep `int`, `float`, `decimal`, `date`, `time` and `datetime` values as such, after a write and a read, without a schema. It completes [NUMERIC_TYPES.md](NUMERIC_TYPES.md) (the numeric types and their keys) and [TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md) (formats and the typed literal grammar), which left this choice open; the maintainer chose C.

Nothing here is implemented. No code of N2, of format 3 or of native values starts before the maintainer approves this design.

Statements about the current engine refer to main at `bc43a47` plus the open PRs #93 (exact numbers) and #94 (date formats); the last section lists what was checked and what was not.

## 1. The answer in short

- **One document model.** Documents stop being `serde_json::Value` inside the engine and become a KivaDB tree, `Doc`, whose leaves are the JSON types plus `int`, `float`, `decimal`, `date`, `time` and `datetime`. Every module that handles documents moves to it (about 520 uses of `serde_json::Value` in about 25 files).
- **One new binary encoding**, `KDOC`, written only for documents that hold a value JSON cannot carry (`decimal`, `date`, `time`, `datetime`). A document of JSON types only is written as today (JSON or MessagePack): `int` and `float` already stay apart there (`8` against `8.0`).
- **Format 3** is raised in a file only by the first record that needs it: a `KDOC` document, or a key of type `int`, `float` or `decimal`. A file that never receives one stays at format 1 or 2, and older versions keep reading it. The raise works as the raise to format 2 does today.
- **Programs** keep receiving plain JSON by default, unchanged for every existing type. A new **extended JSON**, asked for explicitly, carries the types without ambiguity: `{"$decimal": "1250.50"}`, `{"$date": "2026-10-31"}`. The drivers use it to give native objects back without loss.
- **Nothing is converted behind the user's back**: an existing ISO text stays a text, a number stays the number it was, a `decimal` never becomes a `float`.

## 2. The value model

### 2.1 Types

| Type | Range and precision | Comes from (2.0) | Notes |
| :--- | :--- | :--- | :--- |
| `null`, `bool`, `string` | as JSON | as JSON | unchanged |
| `int` | signed 64-bit | JSON integers within `i64` | |
| `float` | IEEE 754 double, finite | JSON numbers with a fraction or an exponent | `NaN` and infinities refused at every write, as today |
| `decimal` | 96-bit integer mantissa, scale 0 to 28: up to about 7.9 × 10^28, 28 to 29 significant digits | new | `rust_decimal` crate (new dependency); keeps its scale: `1250.50` stays `1250.50` |
| `date` | a day, no zone; years 1 to 9999 written as `YYYY-MM-DD` | `Value::Date` (keys) | the range of keys in 2.0 |
| `time` | a time of day, no zone, to the millisecond | `Value::Time` (keys) | `24:00` and leap seconds refused, as #94 does |
| `datetime` | an instant in UTC, to the millisecond | `Value::DateTime` (keys) | zone rules of 2.0 unchanged |
| `array`, `object` | ordered; object keys kept in insertion order, a key given twice keeps the last value | as today (`preserve_order`) | |

`int`, `date`, `time` and `datetime` reuse what keys already have (`Value::Date`, `Value::DateTime`, `Value::Time`, chrono, `src/time.rs`). The same `Doc` leaves are used for keys of these types, so a key and a field of a document hold a date the same way.

**Legacy `u64`.** JSON documents of 2.0 may hold integers between 2^63 and 2^64 (`serde_json` keeps them as `u64`, and #93 accepts them). `int` is `i64`, so such a value is read as a `decimal` with scale 0: exact, typed, and shown with the same digits. A new untyped integer beyond `i64` is refused with the hint `decimal "..."` (the rule of [NUMERIC_TYPES.md](NUMERIC_TYPES.md) 4.1; it replaces the `u64` acceptance of #93 when C lands, and is listed as a change in the release notes).

### 2.2 `decimal`

| Rule | Decision |
| :--- | :--- |
| Precision | 96-bit mantissa, scale 0 to 28 (`rust_decimal`) |
| Written | `decimal "1250.50"` or `decimal 1250.50`, read from its text, never through a float |
| More digits than it holds | refused at write (`decimal "0.12345678901234567890123456789"`), never rounded |
| Scale | kept: `1250.50` is stored, shown and sent as `1250.50` |
| Equality | by value: `1250.5 = 1250.50`; a unique index takes them for one value |
| Order and comparison with `int` | exact |
| Comparison with `float` | exact, by rational arithmetic on the float's exact binary value; never by turning the decimal into a float |
| `+`, `-` (`json incr`, `sum`) | exact; a result beyond the range is refused |
| `*` (`JSON.NUMMULTBY`) | exact when the result fits the range and 28 decimals; otherwise refused, never rounded |
| `avg` | the only operation that rounds: the exact sum divided by the count, rounded half to even at 28 significant digits, documented |
| `decimal` with `float` in arithmetic or `sum` | refused, naming both values: the result would look exact without being so |
| Conversion to `float` | never implicit |

### 2.3 Dates and times

The canonical forms, precision and zone rules of 2.0 stay: a `datetime` is an instant in UTC to the millisecond, read without an offset as a wall clock of the session's zone (skipped hour refused, repeated hour the first); a `date` and a `time` have no zone. #94's formats apply to input and display in the shell only.

## 3. The binary encoding `KDOC`

### 3.1 Where it lives

A record's value is a document payload, as today; a new record flag says it is `KDOC`:

| Bit | Flag | Status |
| :--- | :--- | :--- |
| 0–1 | compression | exists |
| 2 | MessagePack | exists |
| 3 | encrypted | exists |
| **4** | **`FLAG_KDOC`: the document is `KDOC`** | **new, the free bit** |
| 5–7 | batch, commit, tombstone | exist |

Compression and encryption apply on top of the payload, as for JSON and MessagePack. The record CRC covers it, as today. The change journal stores the same type code and bytes as the data file, so its records carry `KDOC` as they are.

### 3.2 Layout

All integers little-endian. Lengths and counts are unsigned LEB128 varints; `int` is a zigzag varint.

```
payload  = version:u8 (= 1) value
value    = tag:u8 body
tag 0x00 null
    0x01 false
    0x02 true
    0x03 int       zigzag varint (i64)
    0x04 float     f64, 8 bytes
    0x05 decimal   16 bytes (rust_decimal::Decimal::serialize: flags, hi, lo, mid)
    0x06 string    varint length, UTF-8 bytes
    0x07 date      i32 days since 1970-01-01, 4 bytes
    0x08 datetime  i64 milliseconds since 1970-01-01T00:00:00Z, 8 bytes
    0x09 time      u32 milliseconds since midnight, 4 bytes
    0x0A array     varint count, values
    0x0B object    varint count, then per member: varint key length, UTF-8 key, value
```

### 3.3 Reading safely

A payload is refused as damaged (`ERR_DATA_CORRUPTED`, naming the key) when:

- its version byte is not one this build knows: "written by a newer KivaDB";
- a tag is unknown;
- a length or a count goes past the end of the payload (lengths are checked before any allocation);
- the nesting is deeper than 128 levels, the limit `serde_json` sets today;
- a string is not UTF-8;
- a date, time or datetime is out of range, a float is `NaN` or infinite, a decimal's scale exceeds 28;
- bytes remain after the value.

Each rule has a test, and the decoder is fuzzed (`cargo fuzz` target, run in CI on a short budget).

### 3.4 When it is written

| Document holds | Written as | Format of the file |
| :--- | :--- | :--- |
| JSON types only (`int` and `float` included), or a `decimal` with scale 0 above `i64::MAX` and up to `u64::MAX`, which JSON reads back as the same `decimal` (15.4) | JSON or MessagePack, as chosen today | unchanged (1 or 2) |
| At least one `decimal`, `date`, `time` or `datetime` | `KDOC` | raised to 3 |

The choice is made per write, on the document as it is. A document that loses its last native value goes back to JSON at its next write. `convert <key> to msgpack` keeps working for JSON-only documents and answers, for a document with native values, that it already is in the binary encoding.

Why not `KDOC` for every document: a file that never holds a native value stays readable by `2.0.0-rc.2` and 1.7.1, so going back stays possible until native values are used.

## 4. Storage format 3

| Decision | Status |
| :--- | :--- |
| `FORMAT_VERSION` becomes 3; `format_for_type` and `format_needed` return 3 for a record of type 11, 12, 13 (`int`, `float`, `decimal` keys, [NUMERIC_TYPES.md](NUMERIC_TYPES.md) 6.1) or with `FLAG_KDOC` | Change |
| The raise rewrites the header's format before appending the record, then syncs, as `Engine::append` does for format 2 today; a crash in between leaves a file of format 3 holding only older records, which 2.0.0 reads | Exists for format 2; reused |
| Compaction copies records with their type, flags and bytes, without decoding them, so it neither raises nor lowers a file's format | Exists |
| `2.0.0-rc.2` and 1.7.1 refuse a file of format 3 at its header: "unsupported storage format 3 (this build reads formats 1 to 2): it was written by a newer KivaDB", and write nothing to it (15.5) | Exists (`parse_file_header`); a test runs the published `rc.2` binary on such a file |
| `del all keys` resets a file to format 1, as today | Exists |

### 4.1 Replication

Replication ships records as type, flags and payload (`Op::Set`). A replica of `rc.2` would refuse an unknown type and could misread `FLAG_KDOC`, so it must never receive them:

| Decision | Status |
| :--- | :--- |
| The replica's handshake gains a capability marker (`native`), like the marker that already refuses 1.x replicas (`ALL_WORKSPACES`, `src/repl.rs`) | Change |
| A 2.0.0 master refuses a replica without it: "upgrade the replica to KivaDB 2.0.0 or later", before any data flows | Change |
| An `rc.2` master and a 2.0.0 replica: the replica follows (it reads formats 1 and 2) | Exists |
| Upgrade order: replicas first, then the master, as the documentation already says for 1.x to 2.0 | Documented |

### 4.2 Backups

| Decision | Status |
| :--- | :--- |
| Archives copy the files as they are; the backup manifest's `format` goes from 1 to 2 when an archive holds a file of storage format 3 | Change |
| An older KivaDB refuses such an archive before restoring anything ("backup format 2 is not supported"), instead of restoring files it then cannot open | Exists (`backup.rs` checks the manifest format) |
| Archives made before 2.0.0 restore in 2.0.0 | Exists |
| `restore --check` of a format-3 archive by 2.0.0 decodes every `KDOC` payload | Change |

### 4.3 Going back

Until a workspace receives a native value, it stays at format 1 or 2, and `rc.2` (and 1.7.1 for format 1) opens it. After that, only a backup made before brings it back to an older version. The release notes and Chapter 33 say so, with the check: `kivadb doctor` reports the storage format of each workspace.

## 5. Behaviour in documents and arrays

| Operation | Rule |
| :--- | :--- |
| Nesting | native values at any depth, in objects and arrays, mixed with other types |
| `get`, `json get` | the shell shows typed leaves in the session's formats; `json type` answers `int`, `float`, `decimal`, `date`, `time`, `datetime` |
| `json set <key> <.path> <value>` | the value keeps its type: `json set o:1 .day date "01-01-2027"` stores a `date`, no longer a text |
| `json incr` | exact on `int` and `decimal`; refuses `decimal` with `float` |
| `json append`, `json insert` | values keep their types |
| Paths | unchanged; a path that ends at a native leaf returns it with its type |
| An existing ISO text | stays a text; nothing converts it. `convert <pattern>.<field> to date` (lot C7) converts explicitly, refusing the documents whose text is not one |

## 6. Comparisons, sorting, indexes, uniqueness, aggregates

### 6.1 Order of kinds

Values sort by kind, then by value. The order of 2.0 is kept, and the temporal kinds go between numbers and texts:

`null` < booleans < numbers (`int`, `float`, `decimal` together, by value) < `date` < `datetime` < `time` < texts

A document without the field, or holding an array or an object there, comes last in both directions, as today.

### 6.2 Rules

| Operation | Rule |
| :--- | :--- |
| Numbers | by exact value across `int`, `float` and `decimal` (#93 for `int` and `float`, extended to `decimal` by rational comparison) |
| Equality | `int 1 = float 1.0 = decimal "1.00"`; a native `date` is not equal to the text `"2026-10-31"` (different types) |
| A temporal literal in a condition | matches native values of its kind **and**, as 2.0 does, ISO texts that read as one: existing data keeps answering the same queries |
| Indexes | key kinds `Num` (exact, from #93, with `decimal`), `Date`, `DateTime`, `Time`. A condition on a temporal literal walks two ranges of an index: the native kind and the canonical texts (`src/engine/ordering.rs`) |
| Unique indexes | duplicates by value within the numeric kind; a `date` and a text are not duplicates, a risk for a field moving from texts to native dates that `schema check` and `doctor` report (15.2) |
| `order by` | the order of 6.1 |
| `group by` | groups by typed value; `month(created_at)` reads native values and ISO texts |
| `sum`, `avg` | over numbers only, by the rules of 2.2 and of [NUMERIC_TYPES.md](NUMERIC_TYPES.md) 4.3 |
| `min`, `max` | any kind, by the order of 6.1 |
| Schemas | `int`, `float`, `decimal` types; `date`, `datetime`, `time` accept native values and, for existing data, ISO texts; `min` and `max` compared exactly |

Indexes live in memory and are rebuilt when a workspace opens: no index file changes.

## 7. JSON for programs

### 7.1 Plain JSON, the default

Every existing type comes out exactly as in 2.0. The new leaves come out as:

| Leaf | Plain JSON |
| :--- | :--- |
| `int` | a JSON number |
| `float` | a JSON number, with a fraction or an exponent (`8.0`) |
| `decimal` | a JSON string: `"1250.50"` (a JSON number would be rounded by most parsers) |
| `date` | `"2026-10-31"` |
| `time` | `"08:30:00"` |
| `datetime` | `"2026-10-31T15:18:00.000Z"` |

Plain JSON loses the type of `decimal` and temporal leaves, never their value. A plain JSON write never creates a native leaf: strings stay strings.

### 7.2 Extended JSON, on request

Asked for with `"types": "extended"` in a structured request, `?types=extended` in REST (`/v1/api`, `/v1/changes`, key reads), and an option of each driver. In both directions:

| Leaf | Extended JSON |
| :--- | :--- |
| `int` | a JSON number without fraction or exponent; its digits are exact, whatever their size within `i64` |
| `float` | a JSON number always written with a fraction or an exponent |
| `decimal` | `{"$decimal": "1250.50"}` |
| `date` | `{"$date": "2026-10-31"}` |
| `time` | `{"$time": "08:30:00"}` |
| `datetime` | `{"$datetime": "2026-10-31T15:18:00.000Z"}` |
| An object of the data whose key starts with `$` | written with the `$` doubled (`"$$ref"`) and read back the same way, so a wrapper is never confused with data |

`$datetime` and `$time` are already the tags of conditions in structured requests; extended JSON reuses them, so a value read in extended JSON can be put in a condition as it is.

### 7.3 Redis, Lua, the changes stream

- **Redis**: `JSON.GET` and the other commands answer plain JSON; RESP has no decimal or date type.
- **Lua**: `int` → Lua integer, `float` → Lua float, `decimal` and temporal leaves → their canonical text. A script writes them with `kiva.decimal("1250.50")`, `kiva.date("2026-10-31")`, `kiva.datetime(...)`, `kiva.time(...)`, which return a typed value.
- **The changes stream** (SSE, `/v1/changes`, the binary protocol): plain JSON by default, extended on request.

## 8. Drivers

### 8.1 Node.js

| Leaf | Default (plain) | `types: "extended"` |
| :--- | :--- | :--- |
| `int` within ±2^53 | `number` | `number` |
| `int` beyond ±2^53 | `number`, rounded by `JSON.parse` (as today) | `BigInt` |
| `float` | `number` | `number` |
| `decimal` | `string` | `string`, or the class given by the application (`decimal: Decimal` from decimal.js or another library), without a dependency in the driver |
| `date`, `time` | `string` | `string` (JS has no date-only or time-only type) |
| `datetime` | `string` | `Date` (millisecond precision, as KivaDB's) |

Reading `BigInt` without loss needs the digits before `JSON.parse` turns them into a `number`. The driver ships a small JSON parser for extended responses (no dependency), since the reviver's access to the source text is not available in every supported Node.js version; which versions offer it is to check. On write, `BigInt` is sent as an exact JSON integer (today `JSON.stringify` throws on one) and refused beyond `i64`.

### 8.2 Python

| Leaf | Default | Extended |
| :--- | :--- | :--- |
| `int` | `int` | `int` (Python integers are exact) |
| `float` | `float` | `float` |
| `decimal` | `str` | `decimal.Decimal` |
| `date`, `time`, `datetime` | `str` | `datetime.date`, `datetime.time`, aware `datetime.datetime` in UTC |

On write, `decimal.Decimal` is accepted (today it is refused), sent as `{"$decimal": ...}`, and refused when it is `NaN`, infinite, or has more digits than a KivaDB decimal holds. `date`, `time` and aware `datetime` inside documents are sent as typed values instead of ISO texts. A naive `datetime` stays refused.

## 9. Compatibility

| Data | Read by 2.0.0 | Written by 2.0.0 |
| :--- | :--- | :--- |
| 1.7.1 files (format 1) | yes: JSON and MessagePack documents become `Doc` (integers → `int`, or `decimal` beyond `i64`; floats → `float`; texts stay texts); `number` keys by the rules of [NUMERIC_TYPES.md](NUMERIC_TYPES.md) 6.1 | stay format 1 until a record needs 2 or 3 |
| `2.0.0-rc` files (format 2) | yes, as above, plus `date`, `datetime`, `time` keys | stay format 2 until a record needs 3 |
| Backups of 1.7.1 and the candidates | restore and open | — |
| Format 3 files, read by `rc.2` or 1.7.1 | refused at the header, nothing is misread | — |
| Accounts, `kiva.conf`, `.meta` | unchanged, except schema types `int`, `float`, `decimal` in `.meta`, which an older version refuses as a damaged file (#90's message); the release notes say so | — |

## 10. Migration

There is no bulk migration: old documents are read as they are, and a file changes format only when a native value is written into it.

| Step | Test |
| :--- | :--- |
| A crash between the header raise and the record | fault injection (the method of #91): the file opens at format 3 with only older records, nothing lost |
| A crash in the middle of a `KDOC` record | the existing end-of-log recovery cuts it, as for any record; CRC test |
| A compaction of a mixed file | records copied byte for byte, format kept |
| Backup of a format-3 workspace, restored by 2.0.0 | `restore --check` decodes every record |
| The same archive restored by `rc.2` | refused by the manifest |
| A format-3 file opened by the published `rc.2` and 1.7.1 binaries | refused with "unsupported storage format 3"; CI downloads them as it already downloads 1.7.1 |
| A replica of `rc.2` on a 2.0.0 master | refused at the handshake |

## 11. Costs

| Cost | Expectation, to be measured |
| :--- | :--- |
| Storage | a `datetime` in 9 bytes instead of 26 (JSON text with quotes), a `date` in 5 instead of 12, a `decimal` in 17 bytes whatever its digits; JSON-only documents unchanged |
| Memory | `Doc` leaves are about the size of `serde_json::Value` ones; `decimal` is 16 bytes |
| CPU | decoding `KDOC` reads lengths instead of parsing text, so it should be faster than JSON; this is not claimed until it is measured |
| Work | the largest piece of 2.0.0 (section 13) |

### 11.1 Benchmarks

Run before and after, on the same machine, published with the machine and the numbers, without promising a gain elsewhere (the rule already followed for 1.7.1 and 2.0):

- size and encode/decode time of a corpus of documents in JSON, MessagePack and `KDOC`: JSON types only, and with dates and decimals;
- writes and reads of documents, single and in transactions;
- index builds at opening, range queries on numbers and dates, unique checks;
- `sum`, `avg`, `group by` on `int`, `float`, `decimal`;
- compaction of a mixed file.

## 12. Risks

| Risk | Likelihood | Impact | Mitigation |
| :--- | :--- | :--- | :--- |
| The document model change breaks a module (about 25 files) | High | High | Lots per layer (section 13), each with the whole test suite; plain JSON output compared byte for byte with 2.0 on a corpus |
| A `KDOC` decoding fault reads damaged data as good | Low | Very high | Strict decoder (3.3), CRC, fuzzing, refusal rather than repair |
| Mixed numeric comparisons wrong at the edges | Medium | High | Property tests against a big-rational reference |
| Old data with ISO texts no longer found by temporal queries | Medium | High | Temporal literals keep matching ISO texts (6.2); tests on 2.0 data |
| A downgrade after native values are written | Certain once written | High | Raise only on need; manifest format; release notes; `doctor` shows the format |
| A replica of `rc.2` misreading records | Certain without a check | Very high | Handshake capability (4.1) |
| Node.js applications rounding integers beyond 2^53 in plain mode | Certain for those values | Medium | Extended mode with `BigInt`; documented |
| The size of the work delays 2.0.0 | High | High | Release candidates until stable; lots that each leave main working |

## 13. Work in pull requests

Each lot is one PR, leaves main working and released as a candidate if wanted, and waits for approval before its code starts.

| Lot | Content | Size | Storage |
| :--- | :--- | :--- | :--- |
| C1 | `src/doc.rs`: the `Doc` tree, `KDOC` encode and decode, exact comparisons, plain and extended JSON, unit and property tests, fuzz target. Not used yet | Large | None |
| C2 | Documents become `Doc` in the engine: `Value::Doc(Doc)`, records with `FLAG_KDOC`, format 3 raised on need, the readers of 1.7.1 and `rc` documents. Plain JSON everywhere, byte for byte as before | Very large | Format 3 |
| C3 | Queries, indexes (new key kinds), sorting, aggregates, schemas, jsonpath, `json incr` on native values | Large | None |
| C4 | The typed literal parser (T3 of [TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md)) producing native leaves; display in the session's formats | Medium | None |
| N2 | `int`, `float`, `decimal` keys on the same leaves (type codes 11 to 13) | Large | Format 3 |
| C5 | Extended JSON in REST, structured requests, the binary protocol and the changes stream; Lua helpers; Redis plain | Medium | None |
| C6 | Drivers: Node.js (`BigInt`, decimal hook, extended parser), Python (`Decimal`, dates) | Medium | None |
| C7 | Replication capability, backup manifest format 2, `doctor` storage format, `convert ... to date`, Chapter 33 and release notes | Medium | None |
| S | Compatibility runs with the published 1.7.1 and `rc.2` binaries, crash tests, benchmarks, documentation, then a candidate prepared from main | Medium | — |

Order: C1 → C2 → C3 → C4 → N2 → C5 → C6, with C7 before any candidate that holds C2, and S last. C2 is the point of no return for files: from it on, a written native value raises format 3.

## 14. What was checked, and how

- In the code of main: the record layout and the free flag bit 4 (`src/format.rs`); the raise to format 2 before the append (`Engine::append`); compaction copying records undecoded (`Engine::compact_impl`); the change journal storing type and value bytes (`src/changes.rs`); replication shipping type, flags and payload (`Op::Set`) and refusing 1.x replicas by a marker (`src/repl.rs`); the backup manifest format check (`src/backup.rs`); the document model in `serde_json::Value` and its uses (counted with `grep`).
- #93 and #94: each passes the full test suite and its CI alone; merged together locally (one trivial conflict in `tests/commands.rs`), the full test suite passes and clippy is clean.
- Not checked: the exact `rust_decimal` behaviours (serialization layout, multiplication and division rounding) beyond its documentation; the Node.js versions whose `JSON.parse` reviver gives the source text; every number of section 11, which are estimates until the benchmarks run.

## 15. Invariants

The properties C1 to C3 must hold, each with its tests. They come from a review of this note against #93, and from the historical data it has to coexist with.

### 15.1 Numbers: one equality, one order, one hash

For any two numeric leaves `a` and `b`, whatever their types among `int`, `float` and `decimal`:

| Invariant | Statement |
| :--- | :--- |
| N-eq | `a = b` exactly when they are the same real number: `int 1 = float 1.0 = decimal "1.00"`, `float 0.5 = decimal "0.50"`, `float -0.0 = int 0` |
| N-ord | the order is total, antisymmetric and transitive, and agrees with N-eq |
| N-hash | `a = b` implies `hash(a) = hash(b)`, so hash indexes and unique checks agree with B-tree ones |
| N-loss | no comparison, sort, hash or index turns one type into another with a loss: a `decimal` is never made a `float`, a large `int` never a `float` |

**Hash.** Each number hashes its canonical class:

1. An integral value within the 128-bit integers hashes as that integer (`int 1`, `float 1.0`, `decimal "1.00"`).
2. Otherwise, a value a decimal holds exactly hashes as the normalized decimal, without trailing zeros (`decimal "0.50"`, `float 0.5`).
3. Otherwise, the value is a float no decimal can equal (`0.1` as a float is 0.1000000000000000055511151231257827…, beyond 28 digits); it hashes as its bits.

`#93`'s `NumKey` already follows rule 1 for `int` and `float`; C1 adds rules 2 and 3 for `decimal`.

**Tests.**

- Property tests over random mixed pairs (including the edges ±2^53, ±2^63, 2^64, `i64::MIN`, the largest decimal, tiny fractions, `-0.0`) check N-eq, N-ord and N-hash against a big-rational reference.
- A unique index fed by a B-tree and by a hash index over the same values gives the same duplicates.

### 15.2 Uniqueness

| Invariant | Statement |
| :--- | :--- |
| U-num | values equal by N-eq are duplicates in a unique index, whatever their types and their encoding (JSON, MessagePack, `KDOC`) |
| U-kind | values of different kinds are never duplicates: a native `date` and the text `"2026-10-31"`, `int 1` and `"1"` |
| U-history | a unique index that holds duplicates written by an earlier version (`0` and `-0.0`, #93) is reported at opening and by `kivadb doctor`; the documents are kept; new duplicates are refused |

**Risk of U-kind.** A field moving from ISO texts to native dates may hold `"2026-10-31"` in one document and `date "2026-10-31"` in another, and a unique index accepts both. To be found:

- `schema check` and `kivadb doctor` report a unique field that mixes texts reading as dates with native dates (lot C7);
- `convert <pattern>.<field> to date` unifies such a field.

### 15.3 Types survive every path

| Path | Invariant | Test |
| :--- | :--- | :--- |
| Write, read, reopen | same type and value | engine round trip |
| Compaction | same bytes | file comparison |
| Backup, restore | same type and value | `backup` then `restore --check`, then reads |
| Replication | the replica reads the same type and value | master and replica in one test |
| Change journal | a change carries the type | `/v1/changes?types=extended` |
| Extended JSON | `Doc` → extended JSON → `Doc` is the identity | property test |
| Drivers, extended mode | Node.js and Python give back the same value and an equivalent type | driver tests against a server |
| Plain JSON | not type-preserving by design: `decimal` and temporal leaves come out as texts | documented, tested as such |

### 15.4 Historical documents

| Invariant | Statement |
| :--- | :--- |
| H-read | a document of 1.7.1 or a candidate reads as it was: integers within `i64` → `int`, floats → `float`, texts stay texts |
| H-u64 | an integer between 2^63 and 2^64 in a JSON document reads as a `decimal` with scale 0, exactly |
| H-format | a document read from JSON or MessagePack and written back without a new native value stays JSON or MessagePack, and its file keeps its format |

H-u64 and H-format together need one rule in section 3.4. A `decimal` with scale 0 and a value above `i64::MAX` and up to `u64::MAX` counts as JSON-encodable: it is written as a JSON integer, and read back from JSON as the same `decimal` by H-u64. Without this rule, a historical document holding such an integer would be rewritten in `KDOC`, and its file raised to format 3, by an unrelated change (`json set` of another field). A `decimal` typed by a user with a value within `i64` (`decimal "5"`) is not JSON-encodable: JSON would read it back as an `int`, so it is written in `KDOC`.

**Tests.**

- The 1.7.1 fixture and an `rc.2` home are opened, queried, written to without native values, and checked to keep their formats.
- A document holding `18446744073709551615` is changed in another field, and its file stays at format 2.

### 15.5 Migration to format 3

| Invariant | Statement |
| :--- | :--- |
| M-need | a file is raised only by a record that needs format 3 |
| M-order | the header is raised and synced before that record is appended; a crash in between leaves a format-3 file of older records only |
| M-never-lower | nothing lowers a file's format except `del all keys`, which empties it |
| M-refuse | `rc.2` and 1.7.1 refuse a format-3 file at its header **and write nothing to it**: its bytes are the same after the attempt |
| M-replica | an `rc.2` replica is refused at the handshake, before any record is sent |
| M-backup | an `rc.2` restore refuses a format-3 archive before writing anything |

M-refuse is checked by running the published `rc.2` and 1.7.1 binaries on a format-3 home, comparing every file before and after, in the way `a_format_from_a_newer_kivadb_is_refused` (`tests/compat.rs`) does today for a newer format.

## 16. Review of this note

Changes made after reviewing it:

- **Section 3.4**: the JSON-encodable rule for `decimal` values in the `u64` range (15.4). Without it, a historical document would raise its file to format 3 through an unrelated change.
- **Section 6.2**: uniqueness across kinds (15.2, U-kind) is a risk now named, with its check and its conversion.
- **Section 4**: M-refuse requires the old versions to leave the file unchanged, not only to refuse it.

Left open, to settle in C1 with tests: the exact `rust_decimal` behaviours this note relies on (the serialization layout, checked overflow, and rounding in division).

## 17. C2 as built

C2 stores documents with native values and reads them back; queries, commands and programs still see plain JSON (C3 to C5).

### 17.1 One change from section 13

Section 13 has documents become `Value::Doc(Doc)`. C2 keeps `Value::Doc(Json)` for documents JSON can hold, and adds `Value::Typed(Doc)` for the others, the ones written in `KDOC`:

- a document is `Typed` exactly when it is stored in `KDOC`: `Value::typed` makes one, and gives back `Doc` in plain JSON when JSON can hold it (15.4, the `u64` rule included);
- JSON and MessagePack documents go through the code of 2.0 unchanged, so H-read and H-format hold by construction, and they pay no conversion;
- the modules that read documents (indexes, vectors, queries, unique checks, schemas, projections, Lua, changes) read a typed one in plain JSON, through `Value::json_doc`;
- the modules that change a document in place refuse a typed one (`json set`, `json incr`, `json del`, `json append`, `json insert`, Redis `JSON.SET` on a path, `JSON.DEL`, `JSON.NUMINCRBY`, `JSON.NUMMULTBY`, `JSON.ARRAPPEND`, `JSON.ARRINSERT`, and a schema that would fill in defaults), so that no read-change-write turns its values into texts. Writing the whole document again works.

C3 can then move the readers to `Doc` one by one, and lift the refusals as each command learns native values.

### 17.2 What is stored

| Piece | Built |
| :--- | :--- |
| Record flag | `FLAG_KDOC` (bit 4); `DocEnc` (`Json`, `MsgPack`, `Kdoc`) carried by the engine, the change journal (flag 32) and the replication stream (byte 2) |
| Format | `FORMAT_VERSION` 3; `format_for_record` gives 3 for a record with `FLAG_KDOC`; raised before the record and synced, as for format 2 |
| Writing | `doc::check` refuses what `decode` would refuse or read differently: nesting deeper than 128, a float not finite, a time or datetime finer than the millisecond, a leap second |
| Reading | a damaged `KDOC` payload is `ERR_DATA_CORRUPTED`, naming the key |
| Compaction | records copied as they are; a record rewritten (recompression, encryption key, transaction flags) keeps `FLAG_KDOC`, which the copy dropped before C2 for flags other than MessagePack |
| Replication | the replica's hello carries `native`; a master refuses a replica without it before any snapshot (`upgrade the replica to KivaDB ... or later`) |
| Backups | manifest format 2 when an archive holds a data file of format 3; `restore --check` decodes every `KDOC` payload that is not encrypted |
| `convert` | a typed document answers that it stays in `KDOC` |

### 17.3 Tests

`tests/native.rs`, 17 tests, among them: the round trip of every type at every depth through writing, reading and reopening; a JSON-only tree written as JSON; the refusals before writing; a 1.7.1 home raised only by a typed document, its values, indexes, workspaces and accounts kept; JSON, MessagePack and `KDOC` documents under one unique index and one B-tree index, rebuilt at opening; a transaction of typed documents cut before its last record; a crash between the header and the record, and in the middle of the record; compaction by copy and by recompression; a damaged payload with a right checksum; backups and restores; the commands that refuse; the change journal; the replication of a record; a replica hello without `native`.

With `KIVADB_RC2_BIN` and `KIVADB_V1_BIN` (the CI downloads both, checked against the checksums of their releases): the published `2.0.0-rc.2` and 1.7.1 refuse a home of format 3 and leave every file as it was, refuse its archive before restoring a file, and a real `rc.2` replica keeps its data and says to upgrade; a home without typed documents still opens in `rc.2`.

### 17.4 Limits

- No command creates a native value yet (C4 typed literals, C5 extended JSON). The engine API (`Value::typed`) and replication are the only writers.
- Encrypted `KDOC` payloads are checked by their checksum only in `restore --check`, which has no key.
- The change journal keeps its version 1: a journal holding `KDOC` changes sits next to a data file of format 3, which an older KivaDB refuses first.
- A Lua script that reads a typed document and writes it back writes texts: Lua gets plain JSON until C5 (`kiva.decimal`, `kiva.date`...).
- Measurements on one Windows laptop ([docs/v2/benchmarks/documents.md](../v2/benchmarks/documents.md)): KDOC about 15 % smaller than JSON for orders with decimals and dates, decoded faster than JSON; the comparison of JSON workloads before and after C2 is within the noise of that machine and is to run again on a quiet Linux machine (lot S).
