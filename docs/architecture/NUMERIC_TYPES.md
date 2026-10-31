# Numeric types: from `number` to `int`, `float` and `decimal` in KivaDB 2.0.0

An audit of the type `number` as KivaDB 2.0 implements it (main at `bc43a47`), then a design to replace it, as a public type, by `int`, `float` and `decimal` before 2.0.0 is released. Nothing here is implemented. Storage changes wait for the maintainer's decision.

Typed literals in documents and arrays, and date formats, are in [TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md); both notes share the grammar of its section 5 and the decision of section 6 below.

## 1. The answer in short

- A `number` **key** keeps the exact text it was written with (`1250.50`, `007`, a 41-digit integer). Everything else works in 64-bit floating point (`f64`): what REST and the drivers receive, numbers inside documents, index keys, equality in conditions, `sum` and `avg`.
- This loses precision **without an error** in four places, checked on the build (section 3): a large integer in a document is rounded; a unique index takes two different integers for duplicates; a condition finds a document that holds another number; `sum` adds in binary floating point.
- `int`, `float` and `decimal` can be added to **keys** with three new type codes and storage format 3, raised only when such a record is written, as 2.0 raised format 2 for dates. Old `number` records stay readable, losslessly.
- In **documents**, `int` and `float` are already told apart: the JSON library keeps `8` and `8.0` apart, on disk too. `decimal` is not: JSON has no decimal. Holding a decimal in a document is the same decision as holding a native date there (section 6), and it is the main decision to take.
- Section 4 fixes the semantics: comparisons by mathematical value across the three types, no silent loss, overflow refused, `NaN` and infinities refused.

## 2. `number` in 2.0: the audit

| Part | In 2.0 | Where |
| :--- | :--- | :--- |
| Type code | `number` is code 2 | `src/value.rs` (`Type::Number`) |
| A key | `Value::Num(String)`: the text as written, if it reads as a finite number: `1250.50`, `007`, `-0`, `12345678901234567890123` are kept as typed; `1e400` is refused (not finite) | `Value::Num`, `is_valid_number` |
| On disk | That text, beside code 2 | `Value::encode` |
| In the shell | The text as stored: `a: 1250.50 [number]` | `Value::display` |
| To programs (REST, structured requests, changes) | `num_to_json`: an `i64` if the text is one, else an `f64`, else the text as a string. `1250.50` becomes `1250.5`; a 23-digit integer becomes `1.2345678901234568e22` | `src/value.rs` |
| From REST | A JSON number or a text, checked by `is_valid_number`, stored as text | `api::value_in` |
| Redis `GET` | The stored text | `src/resp.rs` |
| Redis `INCR`, `INCRBY`, `DECR` | `i64`, overflow refused | `src/resp.rs` |
| Redis `INCRBYFLOAT` | `f64`; `NaN` and infinities refused | `src/resp.rs` |
| Redis `JSON.GET` of a number key | The text read again as JSON | `src/resp_json.rs` |
| Lua | An `i64` text becomes a Lua integer, any other a Lua float; a Lua float comes back as text (`fmt_f64`) | `src/script.rs` |
| Expressions (`print`, computed values) | `f64` | `src/expr.rs` |
| Numbers in documents | `serde_json::Value` without `arbitrary_precision`: each number is an `i64`, a `u64` or an `f64`. `8` and `8.0` stay apart, on disk too; `-0.0` and `18446744073709551615` are kept | checked, section 3 |
| A large integer in a document | Read as an `f64`, so rounded, without an error | checked, section 3 |
| `json incr` | `i64`, overflow refused; with a fraction, `f64`, `NaN` and infinities refused | `jsonpath::incr` |
| Index keys, equality and order in conditions | `SKey::Num(OrdF64)`: every number becomes an `f64` | `src/engine.rs` (`SKey::from_json`) |
| `sum`, `avg` | Added as `f64` | `src/query.rs` |
| Schemas | `number` accepts any JSON number; `integer` any number without a fraction, so `1.0` too | `src/schema.rs` |
| Node.js driver | A JS `number` or `bigint` is sent as `number`; numbers come back as JS numbers (exact up to 2^53) | `drivers/node/index.js` |
| Python driver | `int` and `float` are sent as `number`; they come back as `int` or `float`; `Decimal` is not accepted | `drivers/python/src/kivadb/values.py` |
| Compaction | Copies each record with its type code and bytes, without decoding it | `Engine::compact_impl` |
| Index contents | Rebuilt in memory when a workspace opens; the `.meta` file holds only their definitions | `src/engine.rs` |

## 3. What was checked on the build

Run on the debug build of `bc43a47`, in an empty home:

| Command | Result | Meaning |
| :--- | :--- | :--- |
| `set a 1250.50`, `set b 12345678901234567890123`, `set d 007`, `set e -0`, then `get` | Shown exactly as written, `[number]` | A key keeps its text |
| `set c 1e400` | Refused | Not finite |
| `set big 99999999999999999999999999999999999999999` | Kept, `[number]` | No size limit on a key |
| `set p:1 {"v":12345678901234567890123,"m":1250.50}` | Stored as `{"v":1.2345678901234568e+22,"m":1250.5}` | **Silent rounding in a document** |
| `{"n":8.0,"m":8,"z":-0.0,"u":18446744073709551615}` | Stored as written | `int` and `float` already stay apart in documents |
| `{"x":1e400}` | Refused (`number out of range`) | |
| `sum` of `0.1` and `0.2` | `0.30000000000000004` | Binary floating point |
| Unique index on `q:*.n`, `q:1 {"n":9007199254740992}`, then `q:2 {"n":9007199254740993}` | `q:2` refused: `'q:1' already holds 9007199254740992.0` | **A false duplicate** |
| `get where q:*.n = 9007199254740993` | Finds `q:1`, which holds `9007199254740992` | **A wrong result** |
| Unique index, `q:3 {"n":1.0}`, then `q:4 {"n":1}` | `q:4` refused | `1` and `1.0` count as one value, which section 4 keeps |
| `set number t "42"` | Refused: numbers are not quoted | |

Read in the code, not run: the REST, Redis, Lua and driver conversions of section 2. Not checked: whether the Node.js driver can send a `bigint` at all (`JSON.stringify` refuses a `BigInt`, and `toWire` passes it through unchanged).

## 4. The three types

| Type | Range and precision | Written untyped | Written typed |
| :--- | :--- | :--- | :--- |
| `int` | Signed 64-bit: −9 223 372 036 854 775 808 to 9 223 372 036 854 775 807 | An integer literal: `42`, `-7` | `int 42` |
| `float` | IEEE 754 double; finite only; `-0` stored as `0` | A literal with a fraction or an exponent: `1.75`, `2e-3` | `float 1.75`, `float 8` (stored `8.0`) |
| `decimal` | Exact; up to 28 digits after the point and about 28 significant digits in all (the 96-bit decimal of the `rust_decimal` crate, a new dependency); keeps its scale, so `1250.50` stays `1250.50` | Never: no literal becomes a decimal by itself | `decimal "1250.50"` or `decimal 1250.50`, both read from their text, never through a float |

Not proposed for 2.0.0:

- **`uint`**: a second integer type doubles the comparison and arithmetic cases for one bit of range. A value above `i64` goes in a `decimal`.
- **`bigint`**: a `decimal` with scale 0 holds about 28 digits, enough for identifiers and amounts. Arbitrary size can come later, as a type of its own.

### 4.1 Untyped literals

| Literal | 2.0 | 2.0.0 proposed |
| :--- | :--- | :--- |
| `42` (key) | `number`, text `42` | `int` |
| `1250.50` (key) | `number`, text `1250.50` | `float`, shown `1250.5`. To keep the text, write `decimal "1250.50"` |
| `12345678901234567890123` (key) | `number`, exact | **Refused**: `out of range for int: write decimal "..." to keep it exact, or float ... to round it` |
| `12345678901234567890123` (document) | `f64`, rounded without an error | **Refused**, same message |
| `1e400` | Refused | Refused |
| `8.0` (document) | Stays `8.0` | `float` (unchanged) |

The rule is deterministic, and no literal changes type silently. The price is that a large integer written without a type, accepted today, is refused.

### 4.2 Comparisons, equality, uniqueness, order

| Decision | Status |
| :--- | :--- |
| The three types compare by mathematical value: `int 1 = float 1.0 = decimal "1.00"` | Change for exactness (today everything goes through `f64`) |
| `int` against `float` compared exactly, never by turning the `int` into an `f64`: `9007199254740993 > 9007199254740992.0` | Change: fixes the false duplicate and the wrong result of section 3 |
| `decimal` against `float`: exact, by converting the float to its exact decimal value when it fits, else by sign and magnitude | Change |
| A unique index treats equal values as duplicates whatever their types: `1` and `1.0` collide, as today | Exists; kept |
| Index keys: a numeric key kind with that order, in place of `SKey::Num(OrdF64)`. Indexes are rebuilt when a workspace opens, so nothing on disk changes | Change |
| Numbers still sort before texts and after booleans | Exists; kept |

### 4.3 Arithmetic and aggregates

| Operation | Result |
| :--- | :--- |
| `int` + `int` | `int`; overflow **refused** (as Redis `INCR` and `json incr` do today) |
| `int` + `float` | `float` |
| `decimal` + `int` | `decimal`, exact |
| `decimal` + `float` | **Refused**, naming both: the float is not exact, and the result would look exact |
| `sum` | Exact over `int` (overflow refused) and over `decimal`; `f64` over `float`; a field mixing `decimal` and `float` refuses the sum and names one document of each |
| `avg` | `float` over `int` and `float`; `decimal` over `decimal`, rounded to 28 digits, half to even |
| `min`, `max` | Any mix, by value |
| `NaN`, `±∞` | Refused at every write, as today |
| Division by zero in an expression | Refused |

Redis: `INCR` works on an `int`, `INCRBYFLOAT` on a `float` and, exactly, on a `decimal`. Lua: Lua 5.4 has integers and floats; a `decimal` reaches Lua as a text.

## 5. The public type `number`

| Decision | Status |
| :--- | :--- |
| `int`, `float` and `decimal` are the documented numeric types; the documentation stops presenting `number` | Change |
| `set number n 42` and a REST `{"type": "number"}` keep working and mean "the untyped rule": `int` or `float` according to the literal | Change: an alias, so old commands and scripts keep their meaning |
| In a schema, `number` keeps meaning "any number" (now `int`, `float` or `decimal`), and `integer` keeps accepting `1.0`. New schemas write `int`, `float`, `decimal` | Change, compatible |
| `typeof` answers `int`, `float` or `decimal`, never `number` | **A visible break**: a script that tests `typeof ... = number` must change. To list in the 2.0.0 release notes |
| `bool` accepted as an alias of `boolean`, since it is the usual word | Change |

## 6. Storage

### 6.1 Keys

| Decision | Status |
| :--- | :--- |
| New type codes: `int` 11, `float` 12, `decimal` 13. Their bytes are text, as for every other type (`42`, `1.75`, `1250.50`), so a file stays readable and `encode`/`decode` stay simple | Change |
| A file is raised to **storage format 3** the first time it receives such a record, as 2.0 raises format 2 for dates (`src/format.rs`). From then on, `2.0.0-rc.2` and 1.x refuse it ("unsupported storage format 3"), instead of misreading it | Change |
| A record of code 2 (`number`, from 1.x or a 2.0 candidate) is read without loss: an integer text within `i64` → `int` (`007` → `7`); otherwise → `decimal` when its text fits one exactly (`1250.50`, `2.5E-3`); otherwise (a 41-digit integer, `1e300`) it stays a **legacy number**: its text is kept, shown and sent as today, compared as a `float`, refused in arithmetic until it is written again with a type. The doctor lists such keys | Change |
| A code-2 record stays as it is on disk until the key is written again. Compaction copies records without decoding them (section 2), so it raises no file to format 3 by itself | Exists (compaction); kept |
| No migration command is needed. A doctor check can count the code-2 records left | Change, optional |

Why a legacy fraction becomes a `decimal`, not a `float`: its text was kept exactly, so `decimal` is the only type that loses nothing. Someone who wants a `float` writes the key again.

### 6.2 Documents

`int` and `float` need no storage change in documents: `serde_json` keeps them apart, in JSON and in MessagePack. The changes are in how they are read (4.1) and compared (4.2).

`decimal` (like `date`, `datetime` and `time`) has no place in JSON. There are three ways to hold one in a document:

| | A. Canonical text | E. Internal tag | C. Native document tree |
| :--- | :--- | :--- | :--- |
| Stored as | `"balance": "1250.50"`, a text | `{"$decimal": "1250.50"}` inside the engine, removed when a document leaves it | A KivaDB tree whose leaves may be `int`, `float`, `decimal`, `date`, `datetime`, `time`, in a binary encoding |
| Comparisons and sums | Wrong without help: `"9.50" > "10.00"` as texts. Correct only where a schema says `decimal` and the query reads the schema | Correct where the engine knows the tag | Correct everywhere |
| Indexes | Texts | Need every path (`SKey`, ordering, schema, jsonpath) to know the tags | A numeric key, like 4.2 |
| Seen by programs | A text | Plain JSON (decimal as text); an opt-in typed form for drivers | Plain JSON by default (decimal and dates as texts), an opt-in typed form for drivers |
| A document that holds `{"$decimal": ...}` by chance | — | Taken for a decimal, unless such objects are escaped | — |
| Storage format | Unchanged | Format 3 all the same: 2.0 must not read tags as objects | Format 3 |
| Size of the work | Small | Large | Very large: about 520 uses of `serde_json::Value` in about 25 files (engine, query, schema, jsonpath, Redis JSON, REST, Lua, changes, replication, vector search) |

Today, an amount in a document is safe as an **integer number of the smallest unit** (cents): `int` keeps it exact once 4.2 fixes the comparisons. That is the usual practice, and it needs neither A, E nor C.

### 6.3 Recommendation

**Decided**: option C, native values in documents. Its design is [NATIVE_DOCUMENTS.md](NATIVE_DOCUMENTS.md); lot N1 is #93.

1. **Whatever the decision**: the exact comparisons and index keys of 4.2, and the refusal of rounded integers of 4.1. They fix wrong results of 2.0 and change no storage. They are a fix worth its own PR first.
2. **Keys**: `int`, `float`, `decimal` with format 3 (6.1).
3. **Documents**: the choice between A and C, taken once for decimals and dates together (it is the decision of [TEMPORAL_FORMATS.md](TEMPORAL_FORMATS.md) section 6). E is not recommended: it costs nearly as much as C and keeps the collision problem.
   - If decimals inside documents are needed by 2.0.0, for example for amounts that are not in cents: **C**, since 2.0.0 is the moment a storage break costs least (one break instead of two). It is the largest piece of the release and needs its own design before code.
   - Otherwise: **A** for 2.0.0, decimals in documents as canonical texts checked by a schema type `decimal`, and amounts in cents as `int`; C later, by its own design.

## 7. Risks

| Risk | Likelihood | Impact | Mitigation |
| :--- | :--- | :--- | :--- |
| Scripts that test `typeof` = `number` | Certain for some | Medium | Release notes, `number` kept as an alias everywhere else |
| Large untyped integers refused | Low | Low: a clear error names the fix | The message gives `decimal "..."` |
| Untyped fractions in keys lose their written form (`1250.50` → `1250.5`) | Certain | Low: same value | Documentation; `decimal` to keep the text |
| A file raised to format 3 cannot go back to `rc.2` | Certain once written | High for anyone who downgrades | Release notes; backups before upgrading; compaction raises nothing |
| A replica older than its master refuses format 3 records | Certain | High | Same versions on master and replicas, as for format 2 |
| Mistakes in mixed comparisons (int against float near 2^53, decimal against float) | Medium | High: wrong query results | Tests on the edges (section 8), property tests against a reference |
| `rust_decimal` limits (28 digits) | Low | Medium | Refuse beyond them, never round on write |
| Node.js: integers above 2^53 | Certain for those values | Medium | Return a `BigInt` above 2^53, accept a `BigInt` on write (fixing the send) |
| Option C's size delays 2.0.0 | High if chosen | High | Separate design, its own lots, release candidates until it is stable |

## 8. Tests

- **Literals**: `0`, `-0`, `i64::MIN`, `i64::MAX`, one past each (refused), `007`, `1.0`, `1e3`, `1E-3`, `.5` and `+1` (refused, as today), `1e400` (refused), `decimal` with 28 and 29 digits, `decimal "-0.00"`.
- **Comparisons and indexes**: `9007199254740993` against `9007199254740992` and `9007199254740992.0`; `1`, `1.0`, `decimal "1.00"` equal, colliding in a unique index; order of a mixed column ascending and descending; ranges on an index across types.
- **Arithmetic**: overflow of `int` in `sum`, `INCR`, `json incr`, expressions; `decimal + float` refused; `avg` of decimals rounded half to even; `0.1 + 0.2` as `float` and as `decimal`.
- **Legacy**: a 1.7.1 file and a `2.0.0-rc.2` file with `number` keys (`42`, `1250.50`, `007`, a 41-digit integer, `1e300`), read as `int`, `decimal`, `int`, legacy number, legacy number, with their texts unchanged; the file stays format 1 or 2 after reads and compaction, becomes 3 after a new typed write; `rc.2` refuses it then.
- **Protocols**: REST, structured requests, Redis (`GET`, `INCR`, `INCRBYFLOAT`, `JSON.GET`), Lua, the changes stream, replication; byte-for-byte answers for `int` and `float` where 2.0 already gave them.
- **Drivers**: Node.js numbers, `BigInt` and decimals as texts; Python `int`, `float`, `Decimal`; round trips without loss.
- **Transactions and concurrency**: typed writes in a transaction, `json incr` from two clients at once.
- **Platforms**: Windows, Linux and macOS in CI, as for every change.
- **Benchmarks**: compare queries over numeric indexes before and after 4.2. No gain or loss is claimed before it is measured.

## 9. Work in lots

| Lot | Content | Size | Storage |
| :--- | :--- | :--- | :--- |
| N1 | Exact numeric comparisons and index keys; refusal of rounded integers in documents | Medium | None |
| N2 | `int`, `float`, `decimal` for keys: type codes, format 3, legacy reading, shell, REST, Redis, Lua, schemas, `typeof`, `bool` alias | Large | Format 3 |
| N3 | Drivers: Node.js (`BigInt`, decimals as texts), Python (`Decimal`) | Medium | None |
| N4 | Documents: A (schema type `decimal`), or C (its own design first) | Small (A) / very large (C) | None (A) / format 3 (C) |

N1 can start as soon as it is approved. N2 is the storage change that waits for explicit approval. N4 waits for the decision of 6.3.
