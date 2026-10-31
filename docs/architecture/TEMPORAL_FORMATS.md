# Date formats and typed values in documents: a design for KivaDB 2.1

A design, written before any code, for two requests:

1. let each person write and read dates in the form of their country (`DD-MM-YYYY`, `MM/DD/YYYY`...) instead of ISO 8601 only;
2. let a document literal say the type of a value, as in `{"date": date "10-10-2026"}`.

Nothing here is implemented. Each decision says what **exists** in 2.0 (main at `bc43a47`), what **changes**, and what stays **out of scope**. Section 6 sets out the one decision that weighs more than the rest: whether a document keeps a converted ISO text, or holds a real temporal value.

Every statement about 2.0 was checked in the code or by running the debug build of `bc43a47`; section 10 lists the checks and what was not checked.

## 1. The answer in short

- KivaDB 2.0 already has the types `date`, `datetime` and `time`, one canonical form for each, time zones on three levels (server, account, session), and schemas that check date fields on every write path. 2.1 builds on them and replaces none of them.
- **Formats** are a matter of input and display in the shell. They never change what is stored, nor what REST, the binary protocol, Redis or the drivers send. They are set like the time zone: server, then account, then session.
- **Typed values in a document literal** are a shell syntax, read before the JSON parser, that writes the canonical ISO text. The document stays plain JSON.
- This gives a **typed input**, not a **typed document**: once written, a date in a document is a text that a schema can vouch for, not a value that knows it is a date. Section 6 compares the two. For 2.1 the recommendation is typed input (option A), to evaluate schema-typed reads (option D) next, and to leave native values in documents (option C) to a design and a storage format of their own.

## 2. What 2.0 already has

| Part | In 2.0 | Where |
| :--- | :--- | :--- |
| Types of a key | `date` (a day, no zone), `datetime` (an instant in UTC, to the millisecond), `time` (a time of day, no zone): codes 8, 9 and 10 | `src/value.rs` |
| Their stored form | Text: `2026-10-10`, `2026-10-31T13:30:00.000Z`, `08:00:00`, beside the type code of the record. A file is raised to storage format 2 the first time it holds one, so that 1.x refuses it | `Value::encode`, `src/format.rs` |
| Typed input of a key | `set date born "2004-05-09"`. Without the type, `set ref "2026-10-10"` stores text: KivaDB never turns a text into a date by itself | `exec::parse_value`, DOCUMENTATION 5.8 |
| Typed literals in expressions | `date "..."`, `datetime "..."`, `time "..."`, `now()`, `today()`, `current_time`, with `+ 7 days`, `- 2 hours`... in `set`, `json set` and conditions | `exec::time_expr` |
| Accepted texts | date: `YYYY-MM-DD` only. datetime: RFC 3339, `YYYY-MM-DD HH:MM[:SS[.f]]` with `T` or a space, optionally `Z`, ` UTC`, `+01:00`; without a zone, a wall clock of the session's zone. time: `HH:MM` or `HH:MM:SS[.f]` | `time::parse_date`, `parse_datetime`, `parse_time` |
| Display in the shell | date `2026-10-10`; datetime `2026-10-31 14:30:00 +01:00` in the session's zone, milliseconds only when there are some; time `08:00:00` | `time::show`, `date_text`, `time_text` |
| Time zones, three levels | `config set time.zone ...` (server), `modify user "app" timezone ...` (account, from its next login), `timezone ...` (session); `timezone default` goes back | `src/config.rs`, `src/auth.rs` (`User::timezone`), `src/session.rs`, `exec_admin::cmd_timezone` |
| DST | A wall clock skipped when clocks go forward is refused; one repeated when they go back is read as the first | `Zone::instant` |
| Dates in documents | ISO texts. `json set o:1 .born date "2026-10-10"` writes `"2026-10-10"`, `.at datetime "2026-10-31 14:30:00 +01:00"` writes `"2026-10-31T13:30:00.000Z"` | checked, section 10 |
| Conditions on document dates | A field compares as a moment against `now() - 7 days` or `datetime "..."`, whatever its ISO form. In structured requests: `{"$datetime": ...}`, `{"$time": ...}` | `time::compare_tagged`, DOCUMENTATION 11.1 |
| Sorting and indexes | A datetime text with an offset sorts by its instant, a time `HH:MM` as `HH:MM:SS`, a date as text (ISO order is calendar order) | `src/engine/ordering.rs` |
| Schemas | Field types `date`, `datetime` (with its offset only), `time`; defaults `now()`, `today()`; checked on `set`, `json set`, REST (422), binary protocol, Redis, Lua scripts and triggers | `src/schema.rs`, DOCUMENTATION 11.8 |
| Programs | Always ISO 8601, datetimes in UTC (`2026-10-31T13:30:00.000Z`) | DOCUMENTATION 5.8 |
| Node.js driver | A `datetime` key comes back as a `Date`; a `date` or `time` key as a string; inside a document, everything is text | `drivers/node/index.js` (`fromWire`) |
| Python driver | `datetime` (aware only; a naive one is refused), `date` and `time` keys come back as Python objects; inside a document they are written as ISO text and come back as text | `drivers/python/src/kivadb/values.py` |

Three facts found while checking, which the design must account for (section 10 has the commands):

- **A date text in another form compares wrongly, without an error.** In a document, `"born": "10-11-2026"` is not found by `born > date "2026-10-09"`: it is compared as text, and `"1..." < "2..."`.
- **A tagged object is invisible.** `"born": {"$date": "2026-10-12"}` is not indexed (`2 documents indexed` out of 3) and no date condition matches it: indexes and conditions hold texts, numbers and booleans, not objects (`SKey::from_json`).
- **A datetime text without an offset depends on who reads it.** `"at": "2026-10-31 12:00:00"` matches `at > datetime "2026-10-31 11:45:00Z"` in a session in UTC, and not in one in `Africa/Kinshasa`. The text is read as a wall clock of the reader's zone. Lot 3 removes this for new writes, since a typed literal always writes UTC with `Z`.

## 3. Custom formats

### 3.1 Syntax of a format

| Decision | Status |
| :--- | :--- |
| A format is a pattern of tokens: `YYYY` (year, 4 digits), `MM` (month, 2), `DD` (day, 2), `HH` (hour 00–23, 2), `mm` (minutes, 2), `ss` (seconds, 2), `SSS` (milliseconds, 3), `XXX` (offset: `Z` or `±HH:MM`). Between them, only the separators `-`, `/`, `.`, `:`, a space and `T` | Change |
| A pattern is checked when it is set, not when it is used: a date pattern holds `YYYY`, `MM` and `DD` once each and nothing else; a time pattern holds `HH` and `mm`, then optionally `ss`, then optionally `SSS`; a datetime pattern is a date pattern, a separator, a time pattern, and optionally `XXX` | Change |
| Two-digit years (`YY`), single-digit fields (`D`, `M`), month and day names, 12-hour clocks with AM/PM | Out of scope: names and AM/PM depend on a language, and `YY` is ambiguous by nature |
| A pattern where the year comes first and the day before the month (`YYYY-DD-MM`) is refused | Change: see 3.2, it would collide with ISO |

Examples: `DD-MM-YYYY`, `DD/MM/YYYY`, `MM/DD/YYYY`, `DD.MM.YYYY`, `HH:mm`, `DD/MM/YYYY HH:mm:ss XXX`.

The tokens follow the widely used `YYYY-MM-DD` convention rather than `strftime` (`%d-%m-%Y`), because the people asking for this write `DD-MM-YYYY`, and `MM` against `mm` (month against minutes) is the convention they already know.

### 3.2 Reading: strict, ISO always accepted, no guessing

| Decision | Status |
| :--- | :--- |
| A text is accepted if it matches the pattern in force exactly, or is one of the ISO forms 2.0 accepts (section 2). Nothing else | Change |
| ISO is always accepted, whatever the pattern: a file or a colleague's command in ISO works in every session | Change (today ISO is the only form) |
| No guessing. `05/06/2026` is 5 June under `DD/MM/YYYY` and 6 May under `MM/DD/YYYY`, and refused under `DD-MM-YYYY` (wrong separator). KivaDB never tries a second pattern | Change |
| Why `YYYY-DD-MM` is refused: under it, `2026-03-04` would be both 3 April (pattern) and 4 March (ISO). With that pattern forbidden, a text that starts with four digits is always ISO, and one that starts with two is always the pattern | Change |
| An invalid calendar date is refused, as today: `29-02-2027`, `31-04-2026`, `00-01-2026` | Exists (chrono refuses them); the patterns keep it |
| A refused text names the pattern in force and the ISO form: `'10/11/2026' is not a date under the format DD-MM-YYYY of this session: write 10-11-2026 or 2026-11-10` | Change |
| A datetime whose pattern has no `XXX`, or a text without an offset, is a wall clock of the session's zone; with `XXX`, the offset given wins | Exists for ISO; the patterns keep it |
| A wall clock skipped by DST is refused; a repeated one is read as the first | Exists; the patterns keep it. Section 8 asks whether the repeated hour should be refused instead |
| Milliseconds: `SSS` is exactly three digits. An ISO text with more digits is truncated to the millisecond, as today (`.9999` gives `.999`) | Exists (`time::iso`, `SecondsFormat::Millis`) |

### 3.3 Display

| Decision | Status |
| :--- | :--- |
| The pattern in force is used to show `date`, `datetime` and `time` keys (`get`, `print`), the results of `now()`, `today()` and the date functions | Change |
| A datetime is shown in the session's zone, as today. A pattern without `XXX` shows it without its offset: allowed, but the default keeps the offset, and the documentation says why | Change |
| Texts inside documents are **not** reformatted: the engine does not know a document text is a date (section 6) | Out of scope for option A; possible under option D |
| `stats`, `users`, `audit` keep their current form | Out of scope: they are logs, read by tools too |

### 3.4 Where the formats never apply

- **Storage**: unchanged. The same bytes are written whatever the pattern.
- **Protocols**: REST, the binary protocol, Redis and the changes stream keep ISO 8601, datetimes in UTC.
- **Exports, backups, replication**: unchanged.
- **Files and scripts**: `kivadb run file.kva`, Lua scripts and triggers read and write ISO, unless the file sets a format itself (`format date "DD-MM-YYYY"` at its top). A file must give the same result for every user who runs it; with the session's pattern, `05/06/2026` in a file would be a different day for two colleagues.

## 4. Configuration: server, account, session

Modelled on `timezone`, which already works this way.

| Level | Proposed command | Model in 2.0 | Status |
| :--- | :--- | :--- | :--- |
| Server | `config set time.date_format "DD-MM-YYYY"`, `time.time_format`, `time.datetime_format`; the defaults are the 2.0 forms | `config set time.zone ...` | Change |
| Account | `modify user "app" date format "DD-MM-YYYY"` (and `time format`, `datetime format`), `... none` to go back to the server's; anyone for themselves, an admin for anyone; from the next login | `modify user "app" timezone ...` | Change: three optional fields on `User`, written by `User::to_json`; an account without them reads as today |
| Session | `format` shows the three patterns and where each comes from; `format date "DD/MM/YYYY"` sets one for the session; `format date default` goes back | `timezone`, `timezone <name>`, `timezone default` | Change |
| Priority | session, then account, then server | same as the zone | Change |
| Per workspace or project | not proposed: the stored value does not depend on a format, so the format belongs to whoever reads, not to the data | — | Out of scope |

The names are proposals. `format` is not a shell command in 2.0, so it breaks nothing. A thread-local setting, like the zone (`time::set_session_zone`), lets each session of one process keep its own.

An older KivaDB reading an account written by 2.1 should ignore the new fields: `User::from_json` (`src/auth.rs`) reads the fields it knows by name. A test must confirm it (section 8).

## 5. Typed values in a document literal

### 5.1 The syntax

```
set order:1 {
  "id": "fd54D",
  "names": "mlnda",
  "date": date "10-10-2026",
  "created_at": datetime "2026-10-31T14:30:00Z",
  "opens": time "08:30",
  "due": today() + 30 days
}
```

This is **not JSON**: a JSON parser refuses it (2.0 answers `Invalid JSON literal`). It is the KivaDB shell's document syntax, a superset of JSON. Each typed value is written as in the rest of the shell, type before value (`set born date "..."`, `where at > datetime "..."`), so there is one rule to learn.

| Decision | Status |
| :--- | :--- |
| The literal is one token in 2.0 (`Kind::Json`, read by `serde_json`). A pass before `serde_json` replaces each `date "..."`, `datetime "..."`, `time "..."`, `now()`, `today()`, `current_*`, with its shift, by its canonical ISO text, then hands plain JSON to the parser | Change |
| The pass reads values only, never keys or texts: `"date"` as a key, or `"date \"x\""` inside a text, are left alone | Change |
| The texts are read as in 3.2: the pattern of the session, or ISO | Change |
| A wrong value refuses the whole command, with the field: `order:1.date: '31-02-2026' is not a date` | Change |
| What is written: `"date": "2026-10-10"`, `"created_at": "2026-10-31T14:30:00.000Z"`, `"opens": "08:30:00"`. A typed datetime is always written in UTC with `Z`, so its instant no longer depends on the reader's zone (section 2) | Change |
| The syntax exists in the shell and `.kva` files (and in the Kiva Studio console, if it hands its commands to the shell's parser: to check); not in REST bodies, the binary protocol, Redis or the drivers, which keep plain JSON | Change (shell); out of scope (protocols) |
| A key written `date "date": ...` (type before the key) | Not proposed: it does not match the shell's grammar, and puts a type on the field rather than the value, which is the job of a schema |

### 5.2 Typed input, schema and native value: three different things

| Mechanism | What it guarantees | Lasts |
| :--- | :--- | :--- |
| Typed input (`date "..."`) | This write holds a valid date, in canonical form | The write only. The next write of the field may hold any text |
| Schema (`"born": {"type": "date"}`) | Every write of the pattern, from every path, holds a valid ISO date there | As long as the schema; documents written before it are not checked until their next write |
| Native value (option C, section 6) | The stored value is a date, whoever wrote it and whatever the schema | For good |

## 6. The decision: converted ISO text, or a native value in the document

Typed input converts to a text. Once written, `"2026-10-10"` in a document is a text that looks like a date. Nothing in the document says "this is a date", unless a schema does. That is a decision of architecture, set out here with its alternatives.

### 6.1 The options

- **A. Typed input, ISO text stored** (recommended for 2.1). Sections 3 to 5. No change to storage or protocols.
- **B. Tagged objects in plain JSON**: `"date": {"$date": "2026-10-10"}`. Rejected. Checked on 2.0: such a field is not indexed and no date condition matches it (section 2). Every reader (REST, Redis `JSON.GET`, drivers, Lua, applications) would get an object where it expected a value, and a field holding the text `{"$date": ...}` by chance could not be told apart.
- **C. Native values in documents**: the document becomes a KivaDB tree whose leaves may be `date`, `datetime` or `time`, stored in binary (MessagePack has a timestamp extension, but no date or time of day). 2.0 keeps documents as `serde_json::Value`, so this replaces the document model everywhere it is used.
- **D. Schema-typed reads**: storage stays as in A, but a reader may ask (opt-in) that the fields a schema types as `date`, `datetime` or `time` come back as native objects: a Python `date`, a JS `Date`, a value the shell shows in the session's format. The type comes from the schema in force at read time, not from the document.

### 6.2 Comparison

| | A. ISO text, typed input | C. Native value | D. Schema-typed reads |
| :--- | :--- | :--- | :--- |
| **Queries** | As 2.0: a text compares as a moment against a datetime literal; a date as text, correct while every text is ISO (a `10-11-2026` already stored compares wrongly, without an error) | Compared by type; a text never mistaken for a date | As A on the server |
| **Indexes** | As 2.0: texts | Need a new key kind per temporal type, and a rebuild of existing indexes | As A |
| **Sorting** | As 2.0: canonical texts; a datetime without an offset sorts as text | By value | As A |
| **Conversions** | At write, in the shell only | At write on every path: REST, drivers and Redis must say the type, so each protocol needs a typed form | At read, from the schema |
| **Reading without a schema** | Text, exactly as stored | Typed | Text (nothing to type it with) |
| **Old documents** | Unchanged and readable | Stay texts until rewritten; a field can hold a text in one document and a date in another | Typed if they pass the schema's check, else text: the reader must expect both |
| **Compatibility** | Full: same files, same protocols, 2.0 reads what 2.1 writes | New storage format (3), refused by 2.0 as 2.0 raised format 2 against 1.x; a 2.0 replica cannot follow a 2.1 master; a backup cannot go back | Full on the storage; a read option on the protocols |
| **Storage cost** | `"2026-10-31T13:30:00.000Z"`: 26 bytes with its quotes; a date 12 | A datetime in 8 to 10 bytes, a date in about 4; small next to the document, and the optional compression of records (`src/compress.rs`) already shrinks repeated texts | As A |
| **Drivers** | Unchanged: text in documents, native objects for keys (Node.js: `Date` for a `datetime` key, strings for `date` and `time`) | Both must map every temporal leaf, both ways, in nested documents; JS has no date-only type | An opt-in option per request; types depend on the schema, which an admin may change |
| **Redis, Lua, changes stream** | Unchanged | Each needs a rule to show a native value (as ISO text, losing the type again at the edge) | Unchanged |
| **Size of the work** | Small to medium | Very large: the document model, jsonpath, schemas, indexes, Redis JSON, Lua, REST, changes, both drivers, Studio | Medium |

### 6.3 Recommendation

- **2.1: option A.** It meets the request (write and read in one's own format, a typed literal), changes nothing that exists, and fixes the instant of new datetimes in documents.
- **Next, evaluate D.** It gives applications native objects for the fields that matter, with no storage change, as long as a schema exists. Its risk is that the shape of a read depends on a schema an admin may change, so it must be asked for, never the default.
- **C only by its own design**, if a real need appears that A and D cannot meet (for example, mixing texts that look like dates with real dates in one field). It brings a storage format 3, so it belongs to a major version, or a minor one with an explicit migration.

## 7. Formatting in the drivers

| Decision | Status |
| :--- | :--- |
| For 2.1, the drivers keep sending and receiving ISO, and keep their native objects | Exists; unchanged |
| The default answers of the drivers do not change | Constraint |
| Evaluate, for Node.js and Python, explicit functions that read and write a text with the same patterns as the server: `formatTemporal(value, "DD-MM-YYYY")`, `parseTemporal(text, "date", "DD-MM-YYYY")` in Node.js; `kivadb.format_temporal(value, "DD-MM-YYYY")`, `kivadb.parse_temporal(text, "date", "DD-MM-YYYY")` in Python | Change, optional (lot 4) |
| They run in the application, never ask the server, and never read the server's or the account's format: an application's display must not change because an admin changed a setting | Constraint |
| Their only value over `Intl.DateTimeFormat` or `strftime` is that `DD-MM-YYYY` means exactly the same thing in the shell and the application, with the same strict reading. Without that, they add little; the decision can wait for a user's request | Evaluation |
| The Node.js driver returns `date` and `time` keys as strings, unlike Python. Changing it would change a default answer | Out of scope here; noted for its own decision |

## 8. Tests and edge cases

Each lot comes with its tests; these are the cases they must cover.

**Patterns.** Every token and separator; a pattern without `DD`, with `YYYY` twice, with `YY`, with `YYYY-DD-MM` refused when set; an unknown token refused with the list of tokens.

**Dates.** `29-02-2028` accepted, `29-02-2027` refused; `31-04-2026`, `00-01-2026`, `32-01-2026`, `01-13-2026` refused; `1-1-2026` refused (two digits required); years 0001 and 9999; `05/06/2026` under `DD/MM/YYYY` and under `MM/DD/YYYY` gives two different days, and is refused under `DD-MM-YYYY`; `2026-06-05` accepted under all three, as ISO.

**Times and datetimes.** `24:00` refused; `23:59:60` (decide: refuse); `08:30` under `HH:mm:ss` refused, accepted as ISO; offsets `Z`, `+01:00`, `-05:30`, `+14:00`, `+15:00` refused; with and without `XXX`; three digits of milliseconds and more (truncated, as 2.0 does); the hour skipped in spring in `America/New_York` refused; the repeated hour in autumn (first, or refused: decide); a session in `Africa/Kinshasa` and one in UTC read `datetime "31-10-2026 12:00"` as two instants one hour apart, and both write a `Z` text.

**Configuration.** Session over account over server; `default` and `none`; an account's format taken from its next login; two sessions of one process with two patterns at once; a replica keeps showing in its own sessions' formats.

**Document literal.** A typed value at the top, nested in an object, in an array; a key named `date`; the text `"date \"x\""` untouched; an error naming the field; the result identical to the same document written in ISO (byte for byte); a schema checking it like any other write.

**Files and scripts.** A `.kva` file in ISO gives the same result in sessions with different patterns; a file that sets its own format uses it; a Lua script and a trigger keep ISO.

**Transactions.** A typed write inside `begin`... `commit` holds the converted text; changing the format inside the transaction changes the later commands only; pin whether `now()` and `today()` are taken at the command or at the commit.

**Compatibility.** Every existing document reads and compares as in 2.0; `"born": "10-11-2026"` stays a text; protocol answers byte for byte as in 2.0; an account written by 2.1 opened by 2.0; the storage format unchanged after a session of typed writes.

## 9. Work in lots

| Lot | Content | Size | Risks |
| :--- | :--- | :--- | :--- |
| 1 | The pattern: parse and check it, read and write a date, time or datetime with it; unit tests in `src/time.rs`. No change visible to users | Small to medium | Getting strictness wrong (accepting `1-1-2026`); a regression in the ISO reading, which every path uses: keep the ISO code as it is and add the pattern beside it |
| 2 | The three levels and the `format` command; the pattern used to read typed values and literals, and to show keys and functions; files and scripts in ISO; documentation (5.8, 5.9, a new section) | Medium | The thread-local of a session forgotten on one path (HTTP, Redis): copy the places that set the zone; an admin changing the server format surprises sessions: they keep theirs until their next login, as the zone |
| 3 | The typed document literal: the pass before `serde_json`, its errors, its tests | Medium | Reading inside strings or keys by mistake: write the pass as a small tokenizer that knows JSON strings, not as a search and replace |
| 4 | Driver functions (optional, section 7) | Small | Two copies of the pattern grammar to keep in step: share the test cases with lot 1 |
| 5 | Evaluate option D: a short design, then a decision | Design only | The shape of reads depending on a schema |
| — | Option C | Not planned | Section 6 |

Recommended order: 1, 2, 3, each in its own PR, each with its tests and documentation; 4 and 5 after a decision. No lot touches the storage format or the protocols.

## 10. What was checked, and how

Run on the debug build of main at `bc43a47`, in an empty home:

- `json set o:1 .born date "2026-10-10"`, `.at datetime "2026-10-31 14:30:00 +01:00"`, `.opens time "08:00"` write `"2026-10-10"`, `"2026-10-31T13:30:00.000Z"`, `"08:00:00"`.
- `set o:2 {"d": date "2026-10-10"}` answers `Invalid JSON literal`: typed values inside a literal do not exist yet.
- `set date k1 "10-10-2026"` is refused; `set date k2 "2028-02-29"` accepted; `set date k3 "2027-02-29"` refused.
- With `e:1 {"born":"2026-10-10", ...}`, `e:2 {"born":"10-11-2026", ...}`, `e:3 {"born":{"$date":"2026-10-12"}, ...}`: `create index on e:*.born` indexes 2 documents; `get where e:*.born > date "2026-10-09"` finds `e:1` only.
- `set datetime t "2026-10-31T12:00:00.9999Z"` shows `12:00:00.999`, and `set time u "08:00:00.9999"` shows `08:00:00.999`: truncated, not rounded.
- `"at": "2026-10-31 12:00:00"` matches `at > datetime "2026-10-31 11:45:00Z"` after `timezone UTC`, not after `timezone Africa/Kinshasa`.

Read in the code, not run: the sorting rules (`src/engine/ordering.rs`), the schema checks (`src/schema.rs`), the drivers' conversions (`drivers/node/index.js`, `drivers/python/src/kivadb/values.py`), the storage of temporal keys and format 2 (`src/value.rs`, `src/format.rs`).

Not checked: whether 2.0 opens an account carrying unknown fields; whether `now()` inside a transaction is taken at the command or the commit; the storage sizes of option C, which are estimates.
