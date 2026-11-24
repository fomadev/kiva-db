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
- the modules that change a document in place refuse a typed one (`json set`, `json incr`, `json del`, `json append`, `json insert`, Redis `JSON.SET` on a path, `JSON.DEL`, `JSON.NUMINCRBY`, `JSON.NUMMULTBY`, `JSON.ARRAPPEND`, `JSON.ARRINSERT`, and a schema that would fill in defaults), so that no read-change-write turns its values into texts. Writing the whole document again works. C3 lifts these refusals (section 18).

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
- Measurements on one Windows laptop ([docs/benchmarks/documents.md](../benchmarks/documents.md)): KDOC about 15 % smaller than JSON for orders with decimals and dates, decoded faster than JSON; the comparison of JSON workloads before and after C2 is within the noise of that machine and is to run again on a quiet Linux machine (lot S).

## 18. C3 as built

C3 lets queries, indexes, sorting, aggregates, schemas and partial changes work on typed documents without losing a type. Programs still receive plain JSON (C5).

### 18.1 Reading a document: one view, two forms

`view::DocView` reads a document as it is stored, JSON or a typed tree, and `view::Leaf` is the value at a path. Conditions, indexes, unique checks, `order by`, aggregates, schemas and the filters of vector search all read through them, so a JSON document is never converted and a typed one never projected to JSON before it is judged.

A value of a typed document is judged as follows:

| Value | Judged as |
| :--- | :--- |
| `decimal` | its exact value: `NumKey::Decimal`, compared, ordered and hashed with JSON numbers by `doc::cmp_num` and `doc::num_class` (15.1) |
| `int`, `float` | the JSON number it is, as before |
| `date`, `time`, `datetime` | its ISO text, which the conditions on moments of 2.0 already compare as a moment, and `order by` sorts by its canonical text |
| text, boolean, null | as in JSON |

**Change from section 6.** Temporal values are not a kind of their own: a native date and the ISO text `"2026-10-31"` are one value in a unique index, and they sort among texts. This keeps old data and new data answering the same queries, and turns the U-kind risk (15.2) the other way round: no duplicates appear by mixing texts and native dates. The maintainer approved this rule on 2026-11-02 (#98): it stays for 2.0.0.

### 18.2 Changing a document

`docpath` makes the changes of `jsonpath` on a `Doc`: `set`, `delete`, `append`, `insert`, `incr` and `mul`. The shell (`json set|delete|append|insert|incr`) and Redis (`JSON.SET` on a path, `JSON.DEL`, `JSON.NUMINCRBY`, `JSON.NUMMULTBY`, `JSON.ARRAPPEND`, `JSON.ARRINSERT`) use them for typed documents. JSON documents keep the code of 2.0. A document that loses its last native value is written in JSON again.

`doc::add` and `doc::mul` implement 2.2:

- integers are exact and refused beyond 64 bits; the `u64` integers of 2.0 stay integers;
- a decimal with an integer or a decimal is exact, and a product needing more than 28 decimals is refused rather than rounded;
- a decimal with a float is refused, naming both;
- an increment written with a fraction is read from its text as a decimal when the value it changes is a decimal, so `json incr k .balance -100.25` adds the decimal `-100.25`, not the nearest float. An increment with an exponent (`1e-1`) stays a float, and is refused against a decimal.

### 18.3 Aggregates and schemas

- **`sum` and `avg`.** Over decimals, with the integers next to them, the result is exact. A float among them is refused. `avg` divides the exact sum with `rust_decimal`, rounded to 28 decimals. Both results come out as the decimal's text, as plain JSON carries decimals.
- **`min` and `max`.** Numbers, decimals included, are ordered by exact value.
- **`group by`.** A decimal groups with the JSON number equal to it when one exists (`10.50` with `10.5`); otherwise it groups under `{"$decimal": "..."}`.
- **Schemas** gain the types `int`, `float` and `decimal`:
  - `number` and `integer` accept decimals (an integer decimal for `integer`);
  - `min`, `max` and `enum` compare decimals exactly;
  - `date`, `datetime` and `time` accept native values as well as ISO texts;
  - defaults are filled into typed documents.
- **Compatibility.** An older KivaDB refuses a `.meta` that holds the new type names, as the design said (9).

### 18.4 What reaches programs

| Path | Typed documents |
| :--- | :--- |
| Shell | `json get` and `json type` name `decimal`, `date`, `time` and `datetime`. `get k.path` gives a date, time or datetime as that type, and a decimal as an exact `number` |
| REST, binary protocol, structured requests | plain JSON. Queries and aggregates judge exactly |
| Redis | plain JSON. `JSON.NUMINCRBY` answers a decimal with its exact digits |
| Lua | plain JSON: decimals and moments as texts. A script that reads a typed document and writes it back writes texts until C5 (`kiva.decimal`...) |
| Change journal, replication | the record as written (`KDOC`), so the types of a partial change are kept |

### 18.5 Tests

`tests/native_queries.rs` has 10 tests:

- **Shell:** nested changes keep the other types; exact increments; the refusal of a decimal with a float; reading with types; a return to JSON.
- **Transactions:** commit and rollback.
- **Queries:** equality, ranges, `in`, `order by`, and moments over decimals in KDOC next to numbers in JSON and MessagePack, run with and without a B-tree index and after a reopen.
- **Unique indexes:** `10.50` against `10.5`, `0.1` the decimal against `0.1` the float, and `7` against `7.000`.
- **Aggregates:** exact `sum` and `avg`, `min`, `max`, `group by`, the month of a native date, the refusal of a decimal with a float, and the structured request.
- **Schemas:** `decimal`, `int`, `number`, `date`, `min`, defaults, `schema check`.
- **Changes and replication:** the change journal, and replication of the bytes.
- **Programs:** Redis `JSON.*` over a real RESP connection; REST reads and a structured query over HTTP; the binary protocol; Lua.

`src/docpath.rs` adds unit tests of the changes and of exact arithmetic. `tests/native.rs` now checks that commands change typed documents, where it checked that they refused them.

### 18.6 Limits

- No command creates a native value yet: typed literals arrive in C4, extended JSON in C5.
- Dates, times and datetimes are not a kind of their own in indexes and sorting (18.1).
- A decimal is compared with the value of a condition, which is a JSON number. `where price = 0.1` therefore finds the decimal `0.1` only if the float `0.1` equals it, which it does not. Ranges work. Equality on such decimals needs decimal literals (C4).

## 19. C4: preparation

Nothing of this section is implemented. It prepares lot C4 (typed literals) so that its review starts from written rules; the code waits for the maintainer's approval.

State of main when it was written (`6bd49e8`): C1 to C3 merged (#98), the design of the random functions merged (#99), and lot R0 merged (#100): `util::random_bytes` no longer falls back on the clock, a failure of the system generator is `ERR_RANDOM_UNAVAILABLE` and nothing is written in place of the random bytes. The CI of main (multi-architecture build, Node.js and Python drivers) is green on `e9b6035` and `6bd49e8`.

### 19.1 What C4 must accept

Typed values at any depth, in objects, arrays and sub-documents, mixed with untyped JSON values, by the grammar of [TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md) 5.2:

```
set account:1 {
  "id": uuid "550e8400-e29b-41d4-a716-446655440000",
  "age": int 25,
  "balance": decimal "1250.50",
  "created_at": datetime "2026-10-31T16:18:55Z",
  "values": [int 8, float 0.5, decimal "0.10", date "2026-10-31"],
  "owner": {"since": date "2020-01-15", "limits": [decimal "500", decimal "2500.00"]}
}
```

| Literal | Stored as | Notes |
| :--- | :--- | :--- |
| `int 25` | `int` | refused with a fraction, an exponent, or beyond `i64` |
| `float 0.5` | `float` | `float 8` is the float `8.0`; `NaN` and infinities refused |
| `decimal "1250.50"`, `decimal 1250.50` | `decimal`, scale kept | read from its text, never through a float; more than 28 decimals refused, never rounded (2.2) |
| `date "..."`, `time "..."`, `datetime "..."` | native `date`, `time`, `datetime` | session formats or ISO; a datetime without an offset is a wall clock of the session's zone |
| `uuid "..."` | a **text**, in canonical form (lower case, with dashes) | see 19.2 |
| `string`, `bool`, `null`, `object`, `array`, `number` | as the untyped value | `number` is the untyped rule |

A document whose leaves are all JSON types after parsing (`{"age": int 25}`) is written in JSON, as today (3.4, H-format): `int 8` and `8` write the same integer. A document holding a `decimal`, `date`, `time` or `datetime` is written in `KDOC`, and raises its file to format 3 (section 4).

Read back with `json type`, each path of the example names its type: `id` is a `string`, `age` an `int`, `balance` a `decimal`, `created_at` a `datetime`, and `values` an array of `int`, `float`, `decimal` and `date`.

### 19.2 `uuid`: a checked text, not a type

Decided by the maintainer on 2026-11-02, for 2.0.0:

- `uuid "..."` is accepted in typed literals and in conditions. The text is checked (32 hexadecimal digits, with or without the four dashes; braces and other forms refused) and written in canonical form as a **string**: `uuid "550E8400E29B41D4A716446655440000"` stores `"550e8400-e29b-41d4-a716-446655440000"`.
- No native `uuid` leaf, no `KDOC` tag, no schema type, no change of format. `json type` answers `string`.
- A native `uuid` type (16 bytes, its own tag) is left to a later version. Adding it then needs a `KDOC` tag and its own compatibility rules, which this choice does not prevent.

### 19.3 Exact conditions

C4 accepts typed literals as the value of a condition:

```
get account:* where balance = decimal "1250.50"
get account:* where balance in [decimal "0.10", decimal "1250.50"]
get account:* where balance >= decimal "1000" and age = int 25
```

| Decision | Rule |
| :--- | :--- |
| Carrying | the condition value is the extended form `{"$decimal": "1250.50"}`, as moments already are (`{"$datetime": ...}`, accepted by `query::check_value`); `int` and `float` stay JSON numbers, a `float` always with a fraction |
| Judging | a `decimal` value is `NumKey::Decimal`, compared, ordered and hashed by `doc::cmp_num` and `doc::num_class` (15.1), on JSON, MessagePack and `KDOC` documents alike |
| Indexes | B-tree and hash lookups use the same `NumKey`. Each condition is tested with and without an index, and after a reopen, and must return the same keys |
| Equality | `decimal "1250.50"` equals `decimal "1250.5"`, the JSON integer `1250` never, and a float only when the float is exactly that value |
| `where price = 0.1` | unchanged: `0.1` is the float `0.1`, which is not the decimal `0.1` (18.6). It is **never** converted to a decimal behind the user's back. The documentation of conditions and Chapter 33 show `decimal "0.1"` as the way to find it |
| `where price = float 0.1` | the same as `0.1`, written explicitly |
| `uuid "..."` | the canonical text: it finds the ids written by `uuid "..."` and the texts already in canonical form |
| Structured requests (REST, binary protocol) | `{"$decimal": "..."}` in their conditions comes with extended JSON (C5); until then they take JSON numbers |

### 19.4 Increments with a typed operand

The type of the operand is written, and read the same way on every path:

| Command | Operand |
| :--- | :--- |
| `json incr k .balance decimal "-100.25"` | the decimal `-100.25` |
| `json incr k .ratio float 0.1` | the float `0.1`; refused against a decimal, naming both (2.2) |
| `json incr k .count int 3` | the integer `3` |
| `json incr k .balance -100.25` (untyped) | the rule of C3, `docpath::increment`: read as a decimal when the target is a decimal, otherwise as the JSON number it is; a number with an exponent stays a float |

The untyped rule stays, since scripts written for the candidates use it, and is documented next to the typed form. It is tested on each path that takes an untyped number: the shell, `.kva` files, Redis `JSON.NUMINCRBY` and `JSON.NUMMULTBY` (which have no typed form), and the Lua helpers when C5 adds them. A typed operand never changes kind: a `decimal` stays exact, a `float` stays a float.

### 19.5 Commands and parser

| Decision | Rule |
| :--- | :--- |
| Commands | `set`, `update`, `add`, `json set`, `json append`, `json insert`, `json incr`, the conditions of `get where`, `count`, the aggregates and `del where`, and `.kva` files |
| Not in C4 | REST bodies, the binary protocol, Redis `JSON.SET`, Lua tables, the drivers: plain JSON until C5 and C6 ([TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md) 5.4) |
| Plain JSON first | `serde_json` reads the token first, as today (`exec::json_literal`); only when it fails and the token holds a type word in a value position does the typed parser run. A literal without types gives the same document as before, by construction |
| Parser | recursive descent over the `Kind::Json` token, producing `Doc` directly; never a search and replace, never inside strings or keys |
| Errors | line, column and path: `line 3, column 18 (.values[3]): '31-02-2026' is not a date` |
| Limits | depth 128 (`doc::MAX_DEPTH`) and the existing size limits; a key given twice keeps the last value |
| Display | `get` and `json get` show typed leaves in the session's formats; plain JSON for programs is unchanged |

### 19.6 A change of behaviour, approved

Today `json set o:1 .day date "2026-10-31"` on a JSON document writes the ISO **text** `"2026-10-31"` (`time_expr`, then `to_json`). Section 5 says C4 stores a native `date` instead. On a JSON document of an existing workspace, this rewrites the document in `KDOC` and raises its file to format 3, after which `2.0.0-rc.2` no longer opens it (4.3).

Queries give the same answers either way, since a native date is judged as its ISO text (18.1). The choice is about files and going back:

| Option | Effect |
| :--- | :--- |
| **A. Native, as section 5 says (chosen)** | one rule everywhere: a typed literal is a native value. Listed in the release notes as a change from the candidates |
| B. Text in JSON documents, native in typed ones | no format raise by this command, but the same command gives two types depending on the document, which this design refuses elsewhere |

**Decision.** The maintainer chose option A on 2026-11-17: in C4, a typed literal written by `json set`, `json append` or `json insert` is a native value, in a JSON document as in a typed one. The release notes and Chapter 33 list it as a change from the candidates, with its consequence: the first native value written into a workspace raises its file to format 3, which `2.0.0-rc.2` no longer opens (4.3). A test checks that `json set o:1 .day date "2026-10-31"` on a JSON document stores a native `date`, rewrites the document in `KDOC`, and that `get o:* where day = date "2026-10-31"` finds it as before.

### 19.7 Tests to write

- Round trip of the example of 19.1, and of every type at depths 1, 2 and 128 (the limit), in objects and arrays, through write, read, reopen, compaction, backup and restore, and replication.
- A document whose typed leaves are JSON types only (`int`, `float`, `string`) stays JSON, and its file keeps its format.
- Refusals, each with its line, column and path: a fraction in `int`, a decimal with 29 decimals, an invalid date, a skipped wall-clock hour, a malformed `uuid`, `[int 8, datetime created_at "..."]`.
- A type word inside a string or a key is text: `{"int 8": "decimal 1"}` reads as plain JSON.
- `where balance = decimal "1250.50"`, `in`, ranges, `and` and `or`, with and without a B-tree and a hash index, and after a reopen: the same keys. `where price = 0.1` still does not match the decimal `0.1`.
- `json incr` with `decimal`, `float`, `int` and untyped operands, on decimal, float and integer targets, in the shell and over Redis.
- A unique index over `decimal "10.50"` and the JSON `10.5`: one duplicate (U-num).
- The full suite, clippy, the Node.js and Python driver tests, and the multi-OS CI.

### 19.8 Out of C4

Extended JSON in REST and the protocols, and `kiva.decimal()` in Lua (C5); the drivers (C6); `int`, `float` and `decimal` keys (N2); `convert ... to date`, and the replication and backup checks (C7); the random functions R1 to R4. C4 prepares no candidate, tag or release.

## 20. C4 as built

C4 lets the shell write native values: typed literals at any depth, exact conditions on decimals, and increments with a typed operand. Programs still send and receive plain JSON (C5).

### 20.1 What changed

| Piece | Built |
| :--- | :--- |
| Parser | `src/literal.rs`: recursive descent over the `Kind::Json` token, producing a `Doc`. `exec::parse_doc` gives the token to `serde_json` first and to this parser only when that fails; a literal holding no type word keeps the error of `serde_json` |
| Grammar | the type words of 19.1 and the moments `now()`, `today()`, `current_timestamp`, `current_date`, `current_time`, followed or not by `+ 30 days`, `- 2 hours`... ([TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md) 5.2) |
| Writes | `set`, `update`, `add`, a value given to `rename` and `.kva` files read typed literals through `parse_doc`; `Value::typed` writes a document holding a native value in `KDOC` and the others in JSON |
| `json set`, `append`, `insert` | read a typed value (`date "..."`, `decimal "..."`, `int 7`, `uuid "..."`, a typed document). A value JSON cannot hold changes a JSON document as a typed tree, which is then written in `KDOC` (19.6, option A); other values keep the code of 2.0 |
| `json incr` | `decimal "-100.25"`, `float 0.1`, `int 3`, by the rules of 2.2; the untyped rule of C3 is unchanged |
| Conditions | `decimal "..."`, `int`, `float`, `number`, `uuid`, `string`, `bool` before the value of `=`, `!=`, `<`, `<=`, `>`, `>=`, and typed values in `in [...]`. A decimal travels as `{"$decimal": "..."}` (`num::cond_decimal`), is compared by `NumKey::Decimal`, and is looked up in B-tree and hash indexes by `SKey::from_cond`. `explain` writes it back as `decimal "..."` |
| Display | `get` shows a typed document in the typed syntax, its dates and times in the session's formats: what is shown can be typed back |

### 20.2 Tests

`tests/typed_literals.rs` has 7 tests:

- the example of 19.1 written, read with `json type`, shown and typed back, and reopened; `update`, `add` and an array at the top; the file at format 3;
- a literal of JSON types only (`uuid`, `int`, `float 8`, `string`, `bool`) written in JSON, the file below format 3; a type word inside a text or a key is text;
- refusals with their line, column and path, the plain JSON error kept, and an hour skipped by summer time;
- `decimal "0.1"`, `1250.50`, `float 0.1`, `in`, ranges, `and`, `or`, `!=` and `uuid` conditions, giving the same keys without an index, with a B-tree, with a hash index and after a reopen;
- typed increments, their refusals, the untyped rule, a decimal operand turning a JSON document into `KDOC`, an int operand keeping it JSON;
- `json set`, `append` and `insert` of native values into a JSON document (19.6), and queries answering as before;
- a unique index taking `decimal "10.50"`, `10.5` and `decimal "10.5000"` for one value.

`src/literal.rs` adds unit tests of the grammar, the errors, the depth limit, uuids, scalars and the display.

### 20.3 Limits

- `json type` and `json get` name an `int` or a `float` `number`, as C3 does; the native kinds named are `decimal`, `date`, `time` and `datetime`.
- A literal deeper than 128 levels whose first type word comes after the limit is refused by `serde_json`'s message (recursion limit), not by the typed parser's.
- Structured requests share the check of condition values, so a `{"$decimal": "..."}` there is accepted already; it is documented and tested with extended JSON (C5).
- REST bodies, the binary protocol, Redis `JSON.SET`, Lua and the drivers send plain JSON until C5 and C6.

## 21. C5: preparation

Nothing of this section is implemented. It prepares lot C5 (extended JSON for programs, Lua helpers) so that its review starts from written rules. The decisions of 21.6 are taken.

State of main when it was written (`da48972`): C1 to C4 merged, lot R0 merged, and lot N2 merged (#106): `int`, `float` and `decimal` keys ([NUMERIC_TYPES.md](NUMERIC_TYPES.md) section 11). The CI of main is green on `da48972`.

### 21.1 What reaches programs today, read in the code

| Path | Code | Out | In |
| :--- | :--- | :--- | :--- |
| REST `GET /v1/key/{key}` (and `?path=`), `GET /v1/range` | `http.rs`, `value_json` → `Value::to_json` | plain JSON | `PUT`/`POST /v1/key/{key}`: `put_key`, the body (or the envelope `{"value", "ttl"}`) typed by `Value::from_json` |
| Structured requests: `POST /v1/api`, binary `CMD_API` (8) | `api::handle`; `get` → `value_out`, `query` rows → the document as plain JSON | plain JSON; `"type"` is `Type::name()`: `number` for `int` and `float`, `decimal` for a decimal key | `put`, `add` → `value_in`: an object or array is a `Value::Doc` of plain JSON, so `{"$decimal": "1"}` is stored as an object with a key `$decimal` |
| Conditions of structured requests | `query::check_value`, `SKey::from_cond` | — | `{"$decimal"}`, `{"$datetime"}`, `{"$time"}` already accepted (C4, 20.3) |
| Change journal: `GET /v1/changes`, the `changes` request | `changes::to_json` → `Value::to_json` | plain JSON | — |
| `GET /v1/stream` (SSE), pub/sub events | `http::event_json` | no value, only the event, key and type | — |
| Binary `CMD_GET` (5) and `CMD_SET` (6) | `server.rs`; `Value::encode(false)` | a type byte and the canonical bytes; a typed document as plain JSON; a decimal key as type 13 and its text | `binary_set`: the bytes of a type, documents as plain JSON |
| Redis `JSON.GET`, `JSON.SET`... | `resp_json.rs` | plain JSON | plain JSON: `doc::from_json`, strings stay strings |
| Lua | `script.rs`, `value_to_lua`, `lua_to_value` | a typed document as plain JSON tables (decimals and moments as texts), a decimal key as a text | a table is a `Value::Doc` of plain JSON |
| Node.js driver | `drivers/node/index.js`: `CMD_EXEC` and `CMD_API` only; `fromWire` knows `bytes` and `datetime` | — | — |
| Python driver | `drivers/python/src/kivadb`: `CMD_API`; `from_wire` knows `bytes`, `datetime`, `date`, `time` | — | — |

`doc::to_extended_json` and `doc::from_extended_json` exist since C1 (with a property test: `Doc` → extended JSON → `Doc` is the identity, 15.3) and are used by no path that programs reach. C5 wires them; it designs no new form.

Two facts that the audit adds to section 7:

- The drivers of 1.x use only `CMD_EXEC` and `CMD_API`; `CMD_GET` and `CMD_SET` are used by the Rust `client.rs` (the shell connected to a server). A 1.x client reading a decimal key by `CMD_GET` gets a type byte it does not know (13): N2 made that possible, C5 does not change it, C7 documents it with the replication checks.
- REST and structured requests answer `"type": "number"` for an `int` or `float` key (N2, kept for the 1.x drivers): extended JSON is where the kinds can be named (21.3).

### 21.2 How a program asks for extended JSON

As section 7.2 says, per request, never per connection or by configuration, so that two programs sharing a server or a connection never see each other's choice:

| Path | How |
| :--- | :--- |
| Structured requests (REST `/v1/api`, binary `CMD_API`) | `"types": "extended"` in the request object; `"plain"` or absent is today's answer |
| REST key reads and writes, `/v1/range`, `/v1/changes` | `?types=extended` |
| The `changes` request | `"types": "extended"`, as other requests |
| SSE, pub/sub | nothing to change: they carry no value |
| Binary `CMD_GET`, `CMD_SET` | unchanged: they carry canonical bytes, not JSON |
| Redis | unchanged: plain JSON (7.3); RESP has no decimal or date |

Any other value of `types` is refused (`"types" is "plain" or "extended"`), so that a typo never gives plain JSON silently.

### 21.3 Extended JSON, both ways

Out, with `types=extended`:

- every value is written by `to_extended_json`: a typed document, and also a key of type `decimal`, `date`, `time` or `datetime`, as `{"$decimal": "1250.50"}`...; a `float` with a fraction or an exponent;
- a key of the data that starts with `$` is written with the `$` doubled, in every document, typed or not, so that a program can read any answer with one function;
- `"type"` names the kind of a number key: `int`, `float`, or `number` for a legacy one;
- a legacy number (an integer beyond `i64`, a fraction a float does not hold) is written `{"$decimal": "..."}`: its digits are kept and no JSON parser rounds it. This is a choice to approve (21.6, D3).

In, with `types=extended`:

- `put` and `add` with `"type": "object"` or `"array"` (or no type) read the value with `from_extended_json`: tags become native leaves, `$$` keys become `$` keys, any other `$` key is refused with the message of `from_extended_json`. A document holding a native leaf is stored as `KDOC` and raises the file to format 3, as a typed literal of the shell does (C4);
- `put` with `"type": "decimal"`, `"date"`... also takes the tagged form (`{"$decimal": "1250.50"}`), so that a value read can be written back as it is;
- `PUT /v1/key/{key}?types=extended` reads its body (or the `value` of its envelope) the same way: `{"$decimal": "1250.50"}` alone is a decimal key, `{"$date": ...}` a date key;
- an integer beyond `i64` is refused, with the hint `{"$decimal": "..."}`.

Without `types=extended`, nothing changes in either direction: a `{"$decimal": ...}` object written in plain JSON stays an object with a key `$decimal`, as today. Only conditions read the tags in both modes (C4).

### 21.4 Lua

As 7.3 says: `int` → Lua integer, `float` → Lua number, `decimal` and moments → their canonical text, so that existing scripts read the same values. C5 adds the writers:

| Helper | Returns |
| :--- | :--- |
| `kiva.decimal("1250.50")` | a typed decimal |
| `kiva.date("2026-10-31")`, `kiva.time("08:30:00")`, `kiva.datetime("2026-10-31T15:18:00Z")` | a typed moment, read in ISO only, as extended JSON reads it |
| `kiva.int(n)`, `kiva.float(x)` | a number of that kind (`kiva.float(8)` stays a float) |

A typed value is a Lua userdata: `tostring` gives its canonical text, `==` compares exactly, and it is accepted by `kiva.set` alone or inside a table. A table holding one is stored as a typed document.

The open point is the round trip (D4): a script that reads a typed document, changes one field and writes it back writes texts in place of the decimals and moments it did not touch, since it read texts. Two answers:

- **D4-a, texts (section 7.3 as written).** Reads stay texts; the script must rebuild every typed leaf with the helpers. Simple, and scripts of the candidates are unchanged, but the silent loss of types of C3 (18.4) stays for every script that does not know.
- **D4-b, typed values read as userdata.** A decimal or a moment read from a document is the same userdata that the helpers return; `tostring`, `..` and comparisons with a string see its text, so most scripts are unchanged, and a read-modify-write keeps every type. A script that does arithmetic on a decimal (`d + 1`) or calls `string.*` on it directly changes behaviour: the userdata does arithmetic exactly (decimal + number) or refuses, and `string.*` needs `tostring`.

Recommended: **D4-b**, since keeping types on every path is invariant 15.3 and C3 left Lua as its known exception. The change is listed in the release notes.

### 21.5 Tests to write

- For each path of 21.2: a document holding the four native leaves, an `int` beyond 2^53, a `float` `8.0` and a key `$ref`, read in plain and in extended JSON; written in extended JSON and read back equal; written in plain JSON and stored as plain JSON (format unchanged).
- `types` absent, `plain`, `extended`, and a wrong value (refused).
- Keys of type `decimal`, `date`, `time`, `datetime`, `int`, `float` and a legacy number, in both modes, by `get`, `/v1/key`, `/v1/range` and the change journal.
- Extended writes refused: a `$` key that is not a tag, a tag whose value is not a text, an integer beyond `i64`, a document too deep.
- A value read in extended JSON put back in a condition (`=`, `in`) finds its document, with and without an index.
- Lua: each helper, a typed value inside a table, `tostring` and comparisons; under D4-b, a read-modify-write that keeps a decimal and a datetime untouched, and `d + 1` on a decimal.
- The 1.x drivers, unchanged, against the server of C5: they see plain JSON only (the CI already runs them).
- Format: a plain write leaves a file of format 1 or 2 as it is; an extended write of a native leaf raises it to 3.

### 21.6 Decisions to take

1. **D1, how it is asked**: per request, `"types": "extended"` / `?types=extended`, as 21.2 (section 7.2). Recommended.
2. **D2, writes**: tags are read only in extended requests; plain writes stay plain (21.3). Recommended, since a plain program that stores objects with a `$decimal` key today keeps doing so.
3. **D3, legacy numbers in extended JSON**: `{"$decimal": "..."}` (exact, but its kind changes from `number` to `decimal` when written back), or a bare JSON number (exact text, rounded by most parsers). Recommended: `{"$decimal": "..."}`.
4. **D4, Lua reads**: D4-a texts or D4-b userdata (21.4). Recommended: D4-b.

Taken by the maintainer on 2026-11-18, for 2.0.0: the four recommendations. **D1** extended JSON is asked per request; **D2** tags are read only in extended requests; **D3** a legacy number is written `{"$decimal": "..."}` in extended JSON; **D4-b** Lua reads decimals and moments of documents as typed userdata, and the change is listed in the release notes.

### 21.7 Out of C5

The drivers' options and native objects (C6, and N3 for numbers); `doctor`, `convert ... to date`, the replication and backup checks, and what a 1.x client sees by `CMD_GET` (C7); Redis stays plain JSON. The random functions R1 to R4 are not in C5. C5 prepares no candidate, tag or release; the version stays `2.0.0-rc.2` until a candidate is authorised.

## 22. C5 as built

C5 follows section 21 and the decisions of 21.6. Programs still receive plain JSON unless a request asks for extended JSON; the 1.x drivers are unchanged.

### 22.1 What changed

| Path | Change |
| :--- | :--- |
| Structured requests (REST `/v1/api`, binary `CMD_API`) | `"types": "plain" \| "extended"` (`api::Types`); any other value, or a value that is not a text, is refused. `get`, `query` rows and `changes` answer in extended JSON when asked; `put` and `add` read their value with `doc::from_extended_json` |
| `POST /v1/api?types=extended` | stands for the request's `types` when the body has none; the body's own `types` wins |
| `GET /v1/key/{key}` (and `?path=`), `GET /v1/range`, `GET /v1/changes` | `?types=extended`; a wrong value is a 400 |
| `PUT`/`POST /v1/key/{key}?types=extended` | the body (or the `value` of its envelope) read as extended JSON: `{"$decimal": "9.90"}` alone is a decimal key, a document with native leaves is stored as `KDOC` and raises its file to format 3; `{"$bytes": ...}` stays bytes |
| `"type"` of answers | in extended JSON, the kind of a number key: `int`, `float`, or `number` for a legacy one; plain JSON keeps `number` |
| Legacy numbers (D3) | a key or a number of a plain JSON document that is neither `int` nor `float` comes out `{"$decimal": "..."}` in extended JSON. Plain JSON documents cannot hold an integer beyond `u64` (programs' JSON is refused beyond the 64-bit integers), so the legacy numbers of documents are those between `i64::MAX` and `u64::MAX`. Plain JSON answers them as before |
| Conditions of structured requests | `{"$date": "..."}` is read as its text, which is how a condition carries a date (C4), in both modes, so that every value read in extended JSON can be put in a condition as it is |
| The refusal of an integer beyond the 64-bit integers | names the extended form too: `{"$decimal": "..."}` |
| Lua (D4-b) | a typed document gives its decimals, dates, times and datetimes as typed values (`Typed` userdata); `kiva.decimal`, `kiva.date`, `kiva.time`, `kiva.datetime` (ISO only), `kiva.int`, `kiva.float`; a table holding a typed value is written as a typed document |

### 22.2 Lua typed values

- `tostring`, `print`, `..` and the value a script returns give the canonical text (`1250.50`, `2026-10-31T15:18:00.000Z`); `v:type()` names the type.
- `+`, `-` and `*` are exact between decimals and numbers (`kiva.decimal("0.1") + kiva.decimal("0.2")` is `0.3`) and follow `doc::add` and `doc::mul`; a moment refuses arithmetic; `/` is not defined.
- `<` and `<=` compare with typed values and numbers in the order of section 6.1. `==` compares two typed values exactly (`kiva.decimal("10.50") == kiva.decimal("10.5")`); Lua calls it only between two userdata, so a typed value is never `==` a string or a number: compare with `tostring(v) == "..."` or a typed value.
- `kiva.float(8)` stays a float when written (`8.0 [float]`); `kiva.int` returns a Lua integer and refuses a fraction.

### 22.3 Tests

`tests/extended_json.rs`:

- plain and extended answers of `get` for a typed document, keys of each kind, a legacy key and a plain document with a `$` key and a legacy number; `types` refused when wrong;
- plain writes stay plain (format kept, a `$decimal` key stays a key); extended writes read back equal and raise the file to format 3; `$$` keys, refused `$` keys, a tag holding a non-text, a type mismatch, an integer beyond `i64`; typed scalars written tagged;
- a value read in extended JSON put back in `=` (with and without an index) and in `in`;
- the change journal in both forms;
- REST: `/v1/key` with and without `?types=extended`, `?path=`, a wrong `types`, extended `PUT` of a scalar and of an envelope, plain `PUT` unchanged, `/v1/range`, `/v1/changes`, `/v1/api?types=`;
- Lua: a read-modify-write that keeps every type, texts, exact arithmetic and comparisons, each helper writing a key or a document, errors.

The tests written before C5 are unchanged: plain JSON is what they read.

### 22.4 Limits

- Aggregates (`aggregate`), `similar`, `semantic`, the shell's `changes` command and the SSE events stay plain: they carry results or events, not stored values to write back.
- `CMD_GET` and `CMD_SET` carry canonical bytes, not JSON (21.2); Redis stays plain JSON.
- In Lua, a key of type `decimal`, `date`, `time` or `datetime` is still read as its text (`kiva.get`), as before C5, so a script that writes such a key back as it read it writes a string; D4-b covers the leaves of documents. A key keeps its type when written with the helpers.
- The drivers' options and native objects are C6.

## 23. C6: preparation

Nothing of this section is implemented. It prepares lot C6 (the Node.js and Python drivers read and write native values through extended JSON), so that its review starts from written rules. The decisions of 23.6 are taken.

State of main when it was written (`9428588`): C1 to C5, N2 and R0 merged; C5 (#109) answers extended JSON to requests that ask for it (section 22). The CI of main is green on `9428588`.

### 23.1 The drivers today, read in the code

| | Node.js (`drivers/node`) | Python (`drivers/python`) |
| :--- | :--- | :--- |
| Version, runtimes | `@fomadev/kivadb` 1.2.0; Node.js 18 and later (CI: 18, 20, 22) | `kivadb` 1.0.0; Python 3.10 and later |
| Transport | binary protocol: `CMD_EXEC`, `CMD_API`; `HELLO` gives `KivaDB <version>` | the same |
| Requests | `get`, `set`, `add`, `query`, `del`, `scan`, `similar`, `stats`, semantic cache, `subscribe`, `api` (raw) | the same, plus `explain`, `aggregate`, `changes`, `transaction`, `next_change` |
| Reading | `JSON.parse` of the whole answer: an integer beyond ±2^53 is rounded; `fromWire` makes a `Buffer` of `bytes` and a `Date` of a `datetime` key | `json.loads`: integers exact; `from_wire` makes `bytes`, aware `datetime`, `date`, `time` of keys of those types |
| Writing | `typeOf`: a `BigInt` is typed `number`, but `JSON.stringify` throws on it; a `Date` inside a document becomes its ISO text | `type_of`: `Decimal` is refused; `date`, `time`, aware `datetime` inside a document become ISO texts; a naive `datetime` is refused |
| Conditions (`where`) | sent as given | sent as given: a `datetime` object in a condition makes `json.dumps` fail, so programs write `{"$datetime": ...}` by hand |
| Decimal keys (N2) | `get` gives the text, `type` is `decimal` | the same |

Neither driver sends `types`, so both receive plain JSON from a 2.0 server, exactly as from 1.x: C5 changed nothing for them, which the CI shows.

### 23.2 How a program turns extended JSON on

An option of the connection, as section 8 says:

- Node.js: `connect({ types: "extended" })`; Python: `connect(..., types="extended")`. `"plain"`, the default, keeps every answer of today.
- With it, every request the driver builds carries `"types": "extended"`, and the driver converts both ways (23.3, 23.4). `api()` (the raw request) is sent as the program wrote it and answered as JSON: it stays the escape hatch.
- A server older than 2.0.0 does not know `types` and would answer plain JSON silently. The driver reads the version in `HELLO` and refuses `types: "extended"` before 2.0.0, with the version it found.

### 23.3 Node.js

Reading, with `types: "extended"`:

| Leaf | Value |
| :--- | :--- |
| `int` within ±2^53 | `number` |
| `int` beyond ±2^53 | `BigInt` |
| `float` | `number` |
| `decimal` | `string`, or an instance of the class the program gives (`connect({ types: "extended", decimal: Decimal })`, called as `new Decimal(text)`); the driver depends on no decimal library |
| `date`, `time` | `string` (JavaScript has no date-only or time-only type) |
| `datetime` | `Date` |
| a `$$` key | the `$` key of the data |

`JSON.parse` turns digits into a `number` before any reviver sees them, and the reviver's access to the source text exists only from Node.js 21; the driver therefore ships a small JSON parser, used only for extended answers, that keeps the text of each integer and makes a `BigInt` of it beyond ±2^53. Plain answers keep `JSON.parse`.

Writing, with `types: "extended"`:

- a `BigInt` is an exact JSON integer, refused beyond `i64`, at the top level (a key of kind `int`) and inside a document;
- a `Date` inside a document is `{"$datetime": ...}` (a native datetime) instead of its ISO text;
- an instance of the `decimal` class is `{"$decimal": String(value)}`;
- a key of the data starting with `$` is written with the `$` doubled.

Conditions: in both modes, a `Date` becomes `{"$datetime": ...}`, a `BigInt` an exact integer, an instance of the `decimal` class `{"$decimal": ...}`, since conditions read tags in both modes (C5).

`index.d.ts` gains the option and the types of the values read.

### 23.4 Python

Reading, with `types="extended"`:

| Leaf | Value |
| :--- | :--- |
| `int`, `float` | `int`, `float` (Python integers are exact) |
| `decimal` | `decimal.Decimal` |
| `date`, `time`, `datetime` | `datetime.date`, `datetime.time`, aware `datetime.datetime` in UTC |
| a `$$` key | the `$` key of the data |

Parsing uses `json.loads` with an `object_hook` for the tags; `parse_float` stays `float`.

Writing, with `types="extended"`:

- `decimal.Decimal` is accepted, at the top level (type `decimal`) and inside a document, sent as `{"$decimal": ...}`; `NaN`, infinities and a value with more digits than a KivaDB decimal holds are refused by the driver;
- `date`, `time` and aware `datetime` inside a document are sent as tags; a naive `datetime` stays refused;
- an `int` beyond `i64` is refused with the hint `Decimal(...)`;
- a key of the data starting with `$` is written with the `$` doubled.

In plain mode, `Decimal` stays refused, with the hint `types="extended"`, so that no program stores a decimal as a text without knowing it.

Conditions: in both modes, `Decimal`, `date`, `time` and aware `datetime` objects are written as their tags, so that `client.query("acc:*", {"field": "balance", "cmp": "=", "value": Decimal("10.50")})` works.

### 23.5 Tests to write

- Each driver, against the server of the branch, in both modes: a document holding every native leaf, an `int` beyond 2^53, a `float` `8.0`, a key `$ref`; read back equal to what was written; keys of each kind.
- Plain mode: every test of today unchanged, and a 2.0 server answering exactly as before.
- Node.js: the extended parser against `JSON.parse` on plain values (numbers, escapes, unicode, nesting, errors), `BigInt` at ±2^53 and ±2^63, a `decimal` class given and not given.
- Python: `Decimal` in and out, `NaN` and too many digits refused, naive `datetime` refused, a big `int` refused.
- Conditions with native objects, in both modes, with and without an index.
- `types: "extended"` refused against a server that says 1.x in `HELLO` (a fake server in the test).
- The CI matrix of today (Node.js 18, 20, 22; Python 3.10, 3.12, 3.14).

### 23.6 Decisions to take

1. **E1, the option**: a connection option `types`, `"plain"` by default (23.2). Recommended, since every program of today keeps its answers.
2. **E2, Node.js decimals**: a `string`, or the class the program gives; no dependency in the driver (23.3). Recommended.
3. **E3, Node.js integers**: `BigInt` beyond ±2^53 only, through the driver's own parser (23.3). Recommended; the alternative, `BigInt` for every integer, would change every `number` a program reads.
4. **E4, Python decimals in plain mode**: refused with a hint (23.4). Recommended.
5. **E5, conditions**: native objects written as tags in both modes (23.3, 23.4). Recommended.
6. **E6, versions**: the drivers become Node.js 1.3.0 and Python 1.1.0 (additive, plain by default); nothing is published to npm or PyPI before the maintainer says so.

Taken by the maintainer on 2026-11-18, for 2.0.0: the six recommendations, E1 to E6.

### 23.7 Out of C6

Kiva Studio (its own repository, a condition of `docs/v2/RELEASE.md`); the `changes` request in the Node.js driver; Lua and Redis (C5); `doctor`, the replication and backup checks, and a 1.x client reading a decimal key by `CMD_GET` (C7). C6 prepares no candidate, tag or release.

## 24. C6 as built

C6 follows section 23 and the decisions E1 to E6. Without the new option, both drivers send and read exactly what they did before; their tests of before are unchanged.

### 24.1 Node.js 1.3.0

- `connect({ types, decimal })`: `types` is `"plain"` (default) or `"extended"`, anything else is refused before connecting; `decimal` is a class called `new decimal(text)`. Extended mode is refused when `HELLO` names a version before 2.0.0.
- The requests the driver builds (`get`, `set`, `add`, `query`) go through `_api`: in extended mode it adds `"types": "extended"` and parses the answer with `parseExtended`, a JSON parser of its own that gives a `BigInt` for an integer beyond ±(2^53 - 1) and reads strings with `JSON.parse`; `__proto__` is an own key, as with `JSON.parse`. `api()` sends the request as written and answers with `JSON.parse`.
- Requests are written by `writeJson`, in both modes: a `BigInt` as its digits (refused beyond 64 bits). In a document (`value`) in extended mode, a `Date` is `{"$datetime": ...}`, an instance of `decimal` `{"$decimal": ...}`, and a `$` key is written `$$`; in a condition (`where`) a `Date` and a decimal are tagged in both modes. In a plain document, a `Date` stays its ISO text and a decimal its text.
- A key: a `BigInt` is an `int` in extended mode; an instance of `decimal` is a `decimal`, and is refused in plain mode with the hint `types: "extended"`.
- `index.d.ts` gains `types`, `decimal`, and the type names `int`, `float`, `decimal`.

### 24.2 Python 1.1.0

- `connect(..., types="plain" | "extended")`; anything else is a `ValueError`; extended mode is refused when `HELLO` names a version before 2.0.0.
- `get`, `set`, `add`, `query`, `explain`, `aggregate` and `changes` go through `_api`: in extended mode it adds `"types": "extended"` and reads the answer with `loads_extended` (an `object_hook`: tags to `Decimal`, `date`, `time`, aware `datetime` in UTC; `$$` keys to `$` keys). `api()` is unchanged.
- `values.to_extended` writes documents: `Decimal` (finite, at most 28 digits, written without an exponent), `date`, `time`, aware `datetime` as tags, `$` keys doubled, an `int` beyond 64 bits refused with the hint `decimal.Decimal('...')`. `values.condition` tags the same objects in the conditions of `query`, `explain` and `aggregate`, in both modes.
- `type_of(Decimal)` is `decimal` in extended mode, and a `TypeError` naming `types="extended"` in plain mode.

### 24.3 Tests

- `drivers/node/test/extended.test.js`: the parser against `JSON.parse` (escapes, surrogates, nesting, `__proto__`, errors) and at ±2^53 and 2^63; option checks; a fake 1.7.1 server refusing extended mode; against the server of the branch, reading in both modes, writing documents and keys of each kind, a `$` key, a `BigInt` beyond 64 bits refused, a decimal refused in plain mode, plain documents unchanged, and conditions with a `Date` and a decimal in both modes, with an index.
- `drivers/python/tests/test_extended.py`: `to_extended`, `condition`, `loads_extended` and the refusals; a fake 1.7.1 server; against the server, documents and keys of each kind in both modes, `add`, conditions with and without an index, `aggregate`, and the change journal. `mypy --strict` passes.

### 24.4 Limits

- Node.js reads `date` and `time` as texts (JavaScript has no such types) and writes them as keys through `{ type }`; inside a document, a date or time text stays a text.
- Node.js has no `changes` method yet; `similar`, `scan`, `stats` and the semantic cache answer plain JSON in both modes, as they carry no stored value to write back.
- Nothing is published: the versions are raised in `package.json` and `pyproject.toml`, and npm publishes only on a version tag (the workflow of `drivers/node`), which waits for the maintainer.

## 25. C7: preparation

Nothing of this section is implemented. It prepares lot C7 (what a release needs around native values: checks, conversion, documentation), so that its review starts from written rules. The decisions of 25.6 are taken.

State of main when it was written (`69e9ebf`): C1 to C6, N2 and R0 merged; the `CHANGELOG.md` section of 2.0.0 covers them (#113). The CI of main is green.

### 25.1 What section 13 gives C7, read in the code

| Piece (section 13 and 4) | State |
| :--- | :--- |
| Replication: the `native` capability, a 2.0.0 master refusing an `rc.2` replica before the snapshot (4.1) | **Built in C2** (`repl.rs`: `NATIVE`, checked at the handshake); tested against the published `rc.2` binary (`tests/native.rs`) |
| Backup manifest format 2 for an archive holding a file of format 3; an older KivaDB refusing it (4.2) | **Built in C2** (`backup.rs`, `storage_format`) |
| `restore --check` decoding every `KDOC` record (4.2) | **Built in C2** (`backup.rs`: each `KDOC` payload is unpacked and decoded; an encrypted one is checked by its checksum) |
| `doctor` reporting the storage format of each workspace (4.3) | **Missing**: `doctor` checks the records and the unique indexes (`doctor::data`, `doctor::unique`), not the formats |
| `convert <pattern>.<field> to date` (5, 15.2) | **Missing**: `convert` knows only `to msgpack` and `to json` (`exec::cmd_convert`) |
| `schema check` and `doctor` reporting a unique field that mixes date texts and native dates (15.2) | **Missing**, and its reason changed: C3 judges a native date as its ISO text, so a native date and the text `"2026-10-31"` are one value for a unique index (18.1, `CHANGELOG.md` "Known limits"). The risk U-kind no longer exists for dates; what remains is a field holding both kinds, which programs in extended mode read as a `Date` in one document and a text in another |
| A 1.x client reading a `decimal` key by `CMD_GET` (21.1) | **Not decided**: it gets type 13, which it does not know |
| Chapter 33 and the release notes | **Partly**: the notes come from `CHANGELOG.md`, now complete (#113); Chapter 33 speaks of format 2 only, and the manual documents none of C2 to C6 (no typed literal, `decimal`, extended JSON, Lua helper, driver option) |

### 25.2 `doctor`: the storage format of each workspace

One line per workspace in the "Data" check, from the header of its file, with the oldest version that opens it:

```
  ok    Data        workspace 'default': format 1 (KivaDB 1.x and 2.x)
  ok    Data        workspace 'travel.places': format 3, native values (KivaDB 2.0.0 and later)
```

A summary line counts the workspaces of each format; nothing is a warning, since format 3 is normal for 2.0.0. The text is what Chapter 33 tells an administrator to check before going back (4.3).

### 25.3 `convert <pattern>.<field> to <type>`

So that a field written as ISO texts before 2.0.0 can hold native values, as section 5 says ("nothing converts it; `convert` converts explicitly"):

| Rule | Statement |
| :--- | :--- |
| Types | `date`, `datetime`, `time`, `decimal`: the native types a text stands for. `int` and `float` are already numbers |
| Reading | a `date`, `time` or `decimal` text in ISO only, as extended JSON reads it; a `datetime` text with its offset (`Z` or `+01:00`), since a text without one has no zone to be read in |
| All or nothing | every document of the pattern whose field is a text is checked first; if one does not read as the type, nothing is written and the answer names up to 10 keys with their values. A document whose field is missing, `null`, or already of the type is left as it is |
| Writing | the changed documents are written in one transaction, as typed documents (`KDOC`), raising the file to format 3 |
| Answer | `Converted: 1 204 documents (38 already date, 5 without the field)` |
| Preview | `convert <pattern>.<field> to date check` reads and reports the same counts and the keys that would refuse, and writes nothing |
| Indexes | an index on the field is rebuilt as the documents are rewritten; the answers of queries do not change, since a native date compares as its text (18.1) |
| Rights | as `convert ... to msgpack`: write access to the workspace |

The preview replaces the report of 15.2: `convert ... check` shows how many documents of a field are texts and how many are native, which is the mixed field 15.2 wanted to find, and `doctor` does not read documents.

### 25.4 A 1.x client and a `decimal` key

The 1.x drivers use structured requests and receive a decimal key as its text (`"type": "decimal"`, `"value": "1250.50"`). Only `CMD_GET` (the Rust `client.rs`, that is the shell of 1.x connected to a 2.0 server) receives type 13. The server cannot tell a 1.x client from a 2.0 one: `HELLO` carries no client version. Two answers:

- **document it**: Chapter 33 says that the shell of 1.x connected to a 2.0 server fails on a `decimal` key (a type it does not know), and that the shell of 2.0 reads it;
- or send a decimal key by `CMD_GET` as type `number` (2) with its text: a 1.x client reads it, but loses the type, and the shell of 2.0 needs a new request to see it.

Recommended: **document it**, since the decimal keys are new in 2.0.0 and mixing a 1.x shell with a 2.0 server is a transition case.

### 25.5 Documentation

- **Chapter 33**: format 3 next to format 2 (what raises a file, what refuses it: `rc.2` and 1.x, and `doctor` to check), the changes from the candidates listed in `CHANGELOG.md` (`json set` with a typed value, `int`/`float` instead of `number`, Lua typed values), the order of upgrade of a replication group with `rc.2` replicas, the drivers' option, and going back from format 3 (a backup only).
- **The manual**: the native types in documents and typed literals (the chapter of documents), typed numeric keys (the chapter of types), extended JSON (20.4 and the REST chapter), the Lua helpers, `convert ... to <type>`, the driver option (24.3, 24.4), the error `unsupported storage format 3` in the table of errors.
- Every example of these pages is run by a test, as the manual's examples already are where the tests read them.

### 25.6 Decisions to take

1. **F1, `doctor`**: one line per workspace with its format and the versions that open it (25.2). Recommended.
2. **F2, `convert`**: `to date|datetime|time|decimal`, all or nothing, with `check` to preview (25.3). Recommended; the alternative is to leave old texts as they are and say so in Chapter 33.
3. **F3, the report of 15.2**: replaced by `convert ... check` (25.3). Recommended.
4. **F4, a 1.x client and a `decimal` key**: documented, not changed (25.4). Recommended.
5. **F5, documentation**: Chapter 33 and the manual pages of 25.5 in C7, so that lot S starts from a complete manual. Recommended.

Taken by the maintainer on 2026-11-18, for 2.0.0: the five recommendations, F1 to F5.

### 25.7 Out of C7

The compatibility runs with the published binaries, the crash tests and the benchmarks of a candidate (S); Kiva Studio; the publication of the drivers; the random functions R1 to R4. C7 prepares no candidate, tag or release.

## 26. C7 as built

C7 follows section 25 and the decisions F1 to F5, in two pull requests: the code (#116) and the documentation.

### 26.1 Code

- **`kivadb doctor`** (F1): after the summary of the records, one line per workspace (and `system`) with its storage format and the versions that open it (`doctor::storage_format`, `format_text`). A header that cannot be read is a warning.
- **`convert <pattern>.<field> to date|datetime|time|decimal [check]`** (F2, F3, `exec::convert_field`): each text of the field is read by `doc::from_extended_json`, so in ISO only, as extended JSON reads it; a datetime without an offset is refused. Every document is read first; one refusal stops everything, and the error names up to ten keys with their values and the reason. The changes are written in one transaction, or in the session's own if one is open, as typed documents (`Value::typed`, `KDOC`). Documents without the field, with `null`, or already of the type are counted and left; a value that is not a text (a number) is refused, not guessed. `check` reports the same counts and refusals and writes nothing.
- Tests: `tests/convert_native.rs`.

### 26.2 Documentation

- Chapter 5: numbers named by kind, `decimal` keys, the refusal of untyped big integers, typed values in documents, programs and extended JSON.
- 10.10 (new): typed literals in documents, conditions, storage format 3, `convert`.
- 17.9 `doctor`, 19.4 format 3, 20.4 extended JSON, 21.2 `?types=extended`, 24.3 and 24.4 the drivers' option, 26.2 the Lua helpers and typed values, the table of errors.
- Chapter 33: format 3 in 33.1, `rc.2` replicas in 33.5, the changes for applications in 33.6 (kinds of numbers, big integers, `json set` with a typed value, Lua, the drivers, a 1.x shell and a `decimal` key: F4), going back from format 3 in 33.7.
- The outputs of the shell shown in the manual that changed with N2 (`[number]` to `[int]`, and the bytes of the binary example) are updated; each new example was run against the server of this commit.

### 26.3 Limits

- The rows of `get where` show a typed document as plain JSON; `get <key>` uses the typed syntax.
- `sum` of decimals in the shell showed its total quoted (`Sum: "1250.60"`), since the aggregate gives it as a JSON text. Fixed after C7: `sum`, `avg`, `min` and `max` of decimals are shown as numbers (`Sum: 1250.60`), and a text field keeps its quotes; programs and REST still read the decimal's text, as plain JSON carries decimals.
- The examples of the manual are not yet run by a test of their own (25.5); lot S can add one.

## 27. S: preparation

Nothing of this section is implemented. It prepares lot S, the last of section 13: the checks a candidate needs, then the candidate itself. The decisions of 27.8 are taken.

State of main when it was written (`a97aa83`): C1 to C7, N2 and R0 merged, and the display of decimal aggregates fixed (#118). The CI of main is green. `Cargo.toml` says `2.0.0-rc.2`, the last tag is `v2.0.0-rc.2` (2026-10-29), and nothing written since C2 has been in a candidate.

### 27.1 What section 13 gives S, read in the code

| Piece | State |
| :--- | :--- |
| A home of 1.7.1 opened by main | **Done**: `tests/fixtures/v1.7.1` and `v1.7.1-encrypted`, made by the real 1.7.1 from `v1.7.1.kva`; `tests/compat.rs` and `tests/native.rs` (raised to format 3 only by a typed document) |
| Going back to 1.7.1, and the replication between 1.x and 2.x refused | **Done**, against the real 1.7.1 in the CI (Linux, `KIVADB_V1_BIN`) |
| `rc.2` refusing a home of format 3 and leaving it as it is; an `rc.2` replica refused before the snapshot; a home without typed documents opened by `rc.2` | **Done**, against the real `rc.2` in the CI (Linux, `KIVADB_RC2_BIN`, `tests/native.rs`) |
| A home **written by `rc.2`** opened by main: dates (format 2), projects and grants of 2.0, schemas, the change journal, unique indexes, vectors | **Missing**: there is no fixture of `rc.2`, and the homes of the tests are written by main itself |
| The path of a real upgrade, 1.7.1 then `rc.2` then main, on one home | **Missing** |
| Crash tests | **Partly**: a torn last record, a killed process (`network.rs`), a crash at the first date (`compat.rs`) and at the first typed document (`native.rs`), a transaction cut (`changes.rs`). Not covered: a crash in the middle of `convert ... to <type>`, which writes many typed documents in one transaction and raises the file to format 3 |
| Benchmarks | **Old**: `versions.md` compares 1.7.1 with main at `a15abdb`, before C2 (2026-10-28); `documents.md` measures `KDOC` alone (C2). Neither measures main as it is now, with typed documents, the comparisons of C3 and the exact numbers of N2 |
| The examples of the manual | **Not run**: about 340 `kiva>` lines, none read by a test (25.5, 26.3) |
| The pages of a release | **Stale**: `docs/v2/IMPLEMENTATION_MATRIX.md` has no row for the native values and gives the drivers 1.2.0 and 1.0.0; `docs/v2/RELEASE.md` says the release publishes the Node.js driver 1.2.0 (it is 1.3.0) |
| A candidate prepared from main | **Not started** |

### 27.2 Compatibility with `rc.2`

The users of a candidate run `rc.2` today. What they wrote must open in the next version, and nothing tests that. As for 1.7.1:

- **a fixture** `tests/fixtures/v2.0.0-rc.2`, made once by the published `rc.2` executable (checked against its `SHA256SUMS.txt`) from a script `v2.0.0-rc.2.kva` kept beside it: dates, datetimes and times in keys and documents (format 2), a project with grants and a default workspace, a schema, unique and secondary indexes, an expiry, a vector index, a change journal with a cursor;
- **a test** that opens a copy and finds all of it, reads the journal from its cursor, writes a typed document (format 3) and a `decimal` key, and runs `doctor`, which says format 2 then 3; the fixture itself is never changed, as for 1.7.1;
- **the path of an upgrade** in the CI (Linux, where both old executables are already fetched): a home written by 1.7.1, opened and written by `rc.2`, then opened by main, with `doctor` and every value checked at each step.

### 27.3 Crash tests

One test of its own: `convert order:*.created_at to datetime` on a few hundred documents, the file cut at each record of the transaction (as `a_crash_while_the_first_typed_document_is_written_loses_nothing_else` does), then opened again. Either every document is converted, or none is and the texts are as before; the other keys are never lost. When the header was raised to format 3 before the crash, the file says format 3 with no typed record in it, which an older KivaDB refuses as it would after the conversion; Chapter 33 says so.

### 27.4 Benchmarks

- `benches/versions.rs` run again with main as it is now against 1.7.1, in the same way (3 rounds at 2 000 and 20 000 documents, 2 at 100 000), and `versions.md` rewritten from those runs, the old ones kept in `runs/`.
- One case added: the same orders as typed documents (`KDOC`) and as JSON, written, read and queried, so that the cost of the native values is measured against what they replace.
- The machine: the 2-core Windows laptop of the earlier runs, since it is the one that has them. A run on Linux was wanted (17); the CI runners are shared, and their figures move too much to be published as measurements. The limit stays in `CHANGELOG.md` ("Known limits").

### 27.5 The examples of the manual

A test `tests/manual.rs` that reads `DOCUMENTATION.md` and runs its examples:

- each `text` block whose lines start with `kiva>` is run in a shell session (`Session`, as in `tests/commands.rs`), the blocks of one chapter one after the other in the same home, since an example builds on the one before;
- the answer of each command is compared line by line with the lines under it; `...` in an answer stands for a part that changes (a time, an identifier, a duration);
- a block that cannot run in a test (a server to start, a second machine, the network, a file outside the home, a password typed in) is preceded by `<!-- not run: <reason> -->`, and the test counts the blocks left out, so that the count only goes down by choice;
- first the chapters of the shell, where most examples are (5, 6, 9, 10, 11, 13, 14, 16, 26: about 250 `kiva>` lines); the others follow when a block of theirs changes.

What the test finds wrong is fixed in the manual, or in the code when the manual says what was meant.

### 27.6 The pages of a release

- `IMPLEMENTATION_MATRIX.md`: rows for the native values (C1 to C7, N2, with their tests and sections), the drivers at 1.3.0 and 1.1.0, the benchmarks of 27.4.
- `RELEASE.md`: the Node.js driver 1.3.0, the Python driver 1.1.0, and, in the checklist of a candidate, the runs of 27.2 and the test of 27.5.
- `CHANGELOG.md`: its "Known limits" read again against 26.3 and the decimal aggregates (#118).

### 27.7 The candidate

`RELEASE.md` gives the steps. What S prepares, in a pull request of its own, merged after the others:

- `version = "2.0.0-rc.3"` in `Cargo.toml` and `Cargo.lock`;
- the checklist of `RELEASE.md` gone through, and its result written in the pull request: the CI of main green on the three systems with both old executables, `CHANGELOG.md` complete, the upgrade tried on a copy of a real home of 1.7.1 following Chapter 33.

The tag `v2.0.0-rc.3` is pushed by the maintainer, when the maintainer decides; S pushes none. A candidate publishes neither the drivers nor `latest`, and `kivadb update` keeps 1.7.1 (`RELEASE.md`). Kiva Studio is needed for the release, not for a candidate (`RELEASE.md`, step 4).

### 27.8 Decisions to take

1. **G1, `rc.2`**: a fixture made by the published `rc.2` and its test, and the path 1.7.1 → `rc.2` → main in the CI (27.2). Recommended; the alternative is the path in the CI only, with no fixture kept in the repository.
2. **G2, crash**: the crash in the middle of `convert` (27.3). Recommended.
3. **G3, benchmarks**: main against 1.7.1 again on the laptop, with a case of typed documents; no figure from the CI runners (27.4). Recommended.
4. **G4, the manual**: `tests/manual.rs` on the chapters of the shell, with `<!-- not run -->` and a count of the blocks left out (27.5). Recommended; the alternative is a check by hand before each candidate.
5. **G5, the pages of a release**: updated in S (27.6). Recommended.
6. **G6, the candidate**: `2.0.0-rc.3` prepared from main after G1 to G5, the tag left to the maintainer (27.7). Recommended, since storage format 3 and the drivers' extended mode have reached no user yet; the alternative is `2.0.0` directly, which then needs Kiva Studio first.

Taken by the maintainer on 2026-11-18, for 2.0.0: the six recommendations, G1 to G6.

Pull requests: the tests of G1 and G2; the test of the manual and what it finds (G4); the benchmarks and the pages (G3, G5); the candidate (G6).

### 27.9 Out of S

Kiva Studio (needed by the release, not the candidate); the publication of the drivers on npm and PyPI; the random functions R1 to R4; the pushing of any tag.

## 28. S as built

S follows section 27 and the decisions G1 to G6, in the pull requests listed in 27.8.

### 28.1 Compatibility with `rc.2` and the crash of a conversion (G1, G2)

- **`tests/fixtures/v2.0.0-rc.2`**: a home written by the published `rc.2` for Windows (its `SHA256SUMS.txt` checked) from `tests/fixtures/v2.0.0-rc.2.kva`. It holds dates, datetimes and times as keys (format 2), documents and their indexes (secondary, hash, unique ignoring case), an expiry, an HNSW graph, the project `travel` with two workspaces, a schema with a default, two accounts with grants on the project and a default workspace, and a change journal with a consumer. The journal is kept 3650 days, so that the fixture does not expire. The paths of `kiva.conf` are `@HOME@`, and the lock files and `audit.log` are left out.
- **`tests/compat_rc2.rs`**:
  - this version opens a copy and finds all of it: the indexes are used, the unique index refuses, the graph is read, the schema checks and fills in, and the accounts sign in with their grants;
  - the consumer of the journal reads the 16 changes of `rc.2`, then a change of this version (#17);
  - a typed document, a `decimal` key and `convert ... to decimal` raise the format to 3, workspace by workspace, and everything is there after compaction and reopening;
  - `doctor` gives format 2, then 3, and does not write;
  - the fixture is never changed.
- **The path of an upgrade** (`a_home_goes_from_1_7_1_through_rc_2_to_this_version`): one home is written by the real 1.7.1, then by the real `rc.2` (its accounts migrated, a date: format 2), then by this version (`convert`, `sum` of decimals: format 3). `doctor` reads it, and `rc.2` then refuses it. The test runs when `KIVADB_V1_BIN` and `KIVADB_RC2_BIN` are given, as in the CI on Linux; it was also run on Windows with the published executables.
- **The crash of a conversion** (`tests/convert_native.rs`, `a_crash_in_the_middle_of_a_conversion_converts_everything_or_nothing`): 200 documents are converted, and the file is then cut at 64 places of the transaction and at its last byte. At every cut nothing is converted, the other keys are there and `check` finds nothing damaged. The header stays at 3 with no typed record, as 27.3 says. The whole file holds the 200 conversions.

### 28.2 The examples of the manual (G4)

- **`tests/manual.rs`** runs the `kiva>` examples of chapters 5, 6, 9, 10, 11, 13, 14, 16 and 26: 227 commands, each chapter in a new home, in the time zone of the examples (`time.zone Africa/Kinshasa`).
- **How answers are compared.** The answer of each command is compared line by line with the manual.
  - `...` stands for a part that changes each time, and a line of `...` alone for any number of lines.
  - A time of the clock without an offset (`2026-10-12 09:44:54`, in `scan`, `users` and `audit`) stands for any such time.
  - The folder of the test's home is shown as the manual's (`C:\Users\you\AppData\Roaming\KivaDB`).
- **What an example needs** that the manual says in words, such as the keys of Chapter 9 or the projects of 14.6, is written just before its block, in an HTML comment `<!-- setup ... -->` that the rendered manual does not show.
- **Six blocks are left out**, each with `<!-- not run: <reason> -->`, and the test checks that count:
  - the identifiers of `add`, drawn from the clock;
  - the login prompt;
  - `encryption`, which needs a key at start;
  - the two blocks that load a script from a file;
  - `bench script`.
- **Corrected in the manual**, where the examples did not give what KivaDB answers:
  - the `explain` plans counted 8 keys for the 6 of Chapter 9;
  - `INDEX MERGE` lists the index that gives the fewest keys first (`(age) and (role)`), as the text above it says;
  - 5.9 wrote `born` again, which already existed, so the example now uses another key;
  - 13.4 showed a transaction's status without opening one;
  - 14.2 counted 11 keys in `default`;
  - the `audit` example of 16.9 now shows the commands of 16.6 that come before it;
  - the conversion of 10.10 is now a real one, on four documents.
- **Not covered**: the other chapters (REST, replication, operation), whose examples need a server, a network or a second machine. A block of theirs is added to the test when it changes.
- The manual's conventions say what `...` means in an answer, and that these chapters are run by the tests.

### 28.3 Benchmarks and the pages of a release (G3, G5)

- **`benches/versions.rs`** gains the orders of 27.4. On a server of their own, after the other cases, N/4 orders are written as JSON (a float and a text) and as typed documents (`decimal`, `datetime`), then read, then counted by a condition on the total. Both counts must agree. A version without typed values, 1.7.1, shows none; the table gives it `—`.
- **Run again** on the laptop of the earlier runs: `main` at `7c62009` against the tag `v1.7.1`, both built by `rustc 1.99.0`, at 2 000, 20 000 and 100 000 documents, 3 rounds each. `docs/benchmarks/versions.md` is rewritten from these runs. The earlier runs are kept in `runs/a15abdb`, together with a first run of 100 000 documents in 2 rounds, which one disturbed round made unclear.
- **What it says:**
  - On 2 000 and 20 000 documents, 2.0 runs at the speed of 1.7.1, within the noise of the machine.
  - On 100 000 documents, 2.0 is 8 to 9 % slower to write and read simple values and 5 % slower on transactions, with 7 % more CPU time. That is within the spread of the machine, but in every round but one, so it is written in `versions.md` and in the "Known limits" of `CHANGELOG.md`.
  - Typed orders are written 4 % faster to 10 % slower than JSON orders, read at the same speed, and queried 2 to 6 % faster.
- **Pages:**
  - `IMPLEMENTATION_MATRIX.md`: a section for the native values (C1 to C7, N2, S) with their proofs, the fixture of `rc.2`, the drivers at 1.3.0 and 1.1.0, and the benchmarks.
  - `RELEASE.md`: the checklist of a candidate names `KIVADB_RC2_BIN`, the path through `rc.2` and the test of the manual, and the drivers are at 1.3.0 and 1.1.0.
  - `CHANGELOG.md`: the rows of `get where` and the benchmarks in "Known limits".
- **Not done:** a run on Linux, since the shared CI runners are too noisy to publish (27.4).
