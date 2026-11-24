# KivaDB documentation for contributors

Users read the manual, [DOCUMENTATION.md](../DOCUMENTATION.md), and what each version changes in [CHANGELOG.md](../CHANGELOG.md). This folder holds what explains how KivaDB 2.0 was designed, measured and tested.

## architecture: why it works the way it does

| Page | What it explains |
| :--- | :--- |
| [DECISIONS.md](architecture/DECISIONS.md) | The choices of 2.0 that the manual applies without arguing for them: moments stored as text, time zones, no upgrade command, readiness, the change journal, the order of values and the query plans |
| [NUMERIC_TYPES.md](architecture/NUMERIC_TYPES.md) | `int`, `float` and `decimal` in place of `number` |
| [TEMPORAL_FORMATS.md](architecture/TEMPORAL_FORMATS.md) | Dates, times and datetimes, the formats of each country, typed literals |
| [NATIVE_DOCUMENTS.md](architecture/NATIVE_DOCUMENTS.md) | Typed values inside documents, the `KDOC` encoding and storage format 3, extended JSON |
| [RANDOM_FUNCTIONS.md](architecture/RANDOM_FUNCTIONS.md) | Random values, UUIDs, shuffling, sampling, `order by random()` |
| [PROJECTS_AND_COLLECTIONS.md](architecture/PROJECTS_AND_COLLECTIONS.md) | Projects and collections, their folders, export and import, expiry in the change journal |

## benchmarks: what it costs

| Page | What it measures |
| :--- | :--- |
| [versions.md](benchmarks/versions.md) | 1.7.1 and 2.0 side by side, from outside, through the protocol |
| [stable.md](benchmarks/stable.md) | 2.0.0 against its last candidate, 2.0.0-rc.10 |
| [queries.md](benchmarks/queries.md) | The query planner of 2.0, query by query |
| [documents.md](benchmarks/documents.md) | Typed documents against JSON documents |

Every figure comes with the run that produced it, in [benchmarks/runs](benchmarks/runs). The program is `benches/versions.rs` (`cargo bench --bench versions`).

## testing: what is proved, and what is not

| Page | What it covers |
| :--- | :--- |
| [production-readiness-2.0.0.md](testing/production-readiness-2.0.0.md) | The audit before 2.0.0: each criterion PASS, FAIL or NOT TESTED, with its proof |
| [transaction-crash-audit.md](testing/transaction-crash-audit.md) | What a commit does on its way to the disk, where a crash can stop it, and what a restart must find |
| [transaction-crash-matrix.md](testing/transaction-crash-matrix.md) | Every crash case, what a restart may find, and the test that checks it |
