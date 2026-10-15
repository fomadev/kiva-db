# KivaDB Documentation

The complete manual for KivaDB 1.0.0. It starts from zero: if you have never used a database, begin at Chapter 0 and read in order. If you already know what you are looking for, use the table of contents or jump to the [cheat sheet](#chapter-27--command-cheat-sheet).

Every example in this manual shows what you type after the `kiva>` prompt and, below it, what KivaDB answers.

## Table of contents

* [Chapter 0 — Introduction](#chapter-0--introduction)
* [Chapter 1 — Installation](#chapter-1--installation)
* [Chapter 2 — Starting and stopping KivaDB](#chapter-2--starting-and-stopping-kivadb)
* [Chapter 3 — First steps in the shell](#chapter-3--first-steps-in-the-shell)
* [Chapter 4 — Syntax rules](#chapter-4--syntax-rules)
* [Chapter 5 — Data types](#chapter-5--data-types)
* [Chapter 6 — Writing, reading and deleting](#chapter-6--writing-reading-and-deleting)
* [Chapter 7 — Expiring keys (TTL)](#chapter-7--expiring-keys-ttl)
* [Chapter 8 — Calculations and text with print](#chapter-8--calculations-and-text-with-print)
* [Chapter 9 — Listing keys and ranges](#chapter-9--listing-keys-and-ranges)
* [Chapter 10 — Documents](#chapter-10--documents)
* [Chapter 11 — Queries and indexes](#chapter-11--queries-and-indexes)
* [Chapter 12 — Transactions](#chapter-12--transactions)
* [Chapter 13 — Workspaces](#chapter-13--workspaces)
* [Chapter 14 — Configuration](#chapter-14--configuration)
* [Chapter 15 — Users, roles and security](#chapter-15--users-roles-and-security)
* [Chapter 16 — Maintenance and reliability](#chapter-16--maintenance-and-reliability)
* [Chapter 17 — How data is stored](#chapter-17--how-data-is-stored)
* [Chapter 18 — The TCP server](#chapter-18--the-tcp-server)
* [Chapter 19 — The REST API](#chapter-19--the-rest-api)
* [Chapter 20 — Change notifications](#chapter-20--change-notifications)
* [Chapter 21 — Connecting from your programs](#chapter-21--connecting-from-your-programs)
* [Chapter 22 — Replication](#chapter-22--replication)
* [Chapter 23 — Lua scripting](#chapter-23--lua-scripting)
* [Chapter 24 — Monitoring](#chapter-24--monitoring)
* [Chapter 25 — Using KivaDB inside a Rust program](#chapter-25--using-kivadb-inside-a-rust-program)
* [Chapter 26 — Troubleshooting](#chapter-26--troubleshooting)
* [Chapter 27 — Command cheat sheet](#chapter-27--command-cheat-sheet)
* [Chapter 28 — Limits and scope](#chapter-28--limits-and-scope)
* [Chapter 29 — Deploying KivaDB on a server](#chapter-29--deploying-kivadb-on-a-server)
* [Chapter 30 — Vector search for AI](#chapter-30--vector-search-for-ai)
* [Chapter 31 — Redis compatibility](#chapter-31--redis-compatibility)
* [Chapter 32 — Keeping values in memory (value cache)](#chapter-32--keeping-values-in-memory-value-cache)
* [Glossary](#glossary)

---

## Chapter 0 — Introduction

### What is a database?

A program usually forgets everything when it stops. A **database** is the piece of software that remembers: you hand it information, it writes that information to disk in a safe way, and gives it back whenever you ask, even after the computer has been restarted.

### What is KivaDB?

KivaDB is a **key-value and document database**:

* Every piece of information is stored under a name, called a **key**. `age` is a key; `user:1` is a key.
* What is stored under a key is its **value**. A value can be simple (a number, a piece of text) or structured (a list, or a **document** with named fields such as a user profile).

```text
key        value
---------  ------------------------------------------
age        30
name       "Fordi"
user:1     {"name": "Alice", "role": "admin"}
```

You talk to KivaDB with short commands: `set` to store, `get` to read, `del` to delete, and so on.

### What makes it a real database

* **Types.** KivaDB knows whether a value is a number or a text and refuses to mix them up by accident.
* **Durability.** What you store survives a crash. Each record on disk carries a checksum, so damaged data is detected instead of being returned as if it were good.
* **It runs as a service.** You *start* KivaDB once; it then listens on a network port (2911 by default). Any number of shells, applications and scripts can work on the same data at the same time.
* **Transactions.** Several changes can be grouped so that either all of them happen or none.
* **Multi-user.** Accounts, passwords, roles and an audit log.
* **Several models in one.** Plain key-value, JSON documents with indexes and queries, and vector search for AI embeddings, all in the same keys and the same files. Applications written for Redis can connect to it too.

### How this manual is organised

| Chapters | Subject |
| :--- | :--- |
| 0 – 3 | Getting started |
| 4 – 12 | The command language, from the simplest command to transactions |
| 13 – 17 | Administration: workspaces, configuration, users, maintenance, files |
| 18 – 22 | Networking: protocols, REST, client programs, replication |
| 23 – 25 | Extending: Lua scripts, monitoring, embedding |
| 26 – 28 | Troubleshooting, cheat sheet, limits |
| 29 | Deploying on Linux, Windows, macOS and Docker |
| 30 – 32 | Vector search, Redis compatibility, the value cache |

---

## Chapter 1 — Installation

### 1.1 What you need

* **Rust** (stable). Install it from [rustup.rs](https://rustup.rs).
* **A C compiler.** It is used once, to compile the Lua interpreter that KivaDB embeds. On Windows the "Build Tools for Visual Studio" installed with Rust are enough; on Linux install `gcc`; on macOS install the Xcode command line tools.

### 1.2 Build

From the project folder:

```bash
cargo build --release
```

The first build downloads the dependencies and takes a few minutes. The result is one file:

| System | File |
| :--- | :--- |
| Windows | `target\release\kivadb.exe` |
| Linux, macOS | `target/release/kivadb` |

### 1.3 Make it available everywhere

Copy that file into a folder that is in your `PATH` (or add `target/release` to your `PATH`). After that you can type `kivadb` from any folder.

Check that it works:

```bash
kivadb --version
```

```text
KivaDB 1.0.0
```

### 1.4 Where KivaDB keeps its files

KivaDB has one **home directory** that holds its configuration, its users and its data:

| System | Home directory |
| :--- | :--- |
| Windows | `%APPDATA%\KivaDB` (for example `C:\Users\you\AppData\Roaming\KivaDB`) |
| Linux, macOS | `~/.kivadb` |

It is created automatically the first time you run KivaDB. Because the home is always the same, you find your data again whatever folder you are in.

To use another location, either pass `--home <folder>` on the command line or set the `KIVADB_HOME` environment variable. This is handy for experiments:

```bash
kivadb --home ./scratch-home
```

### 1.5 Run the tests (optional)

```bash
cargo test
```

The test suites check storage, crash recovery, every command, the network protocols and replication.

---

## Chapter 2 — Starting and stopping KivaDB

A database is meant to be **running**: started once, then used by whoever needs it. KivaDB gives you three commands for that.

### 2.1 Start

```bash
kivadb start
```

```text
KivaDB started (pid 21040).
  TCP port : 2911
  Home     : C:\Users\you\AppData\Roaming\KivaDB
  Log      : C:\Users\you\AppData\Roaming\KivaDB\server.log
Open a shell with 'kivadb', stop the database with 'kivadb stop'.
```

KivaDB now runs in the background. You can close the terminal; the database keeps running. It listens on two ports:

| Port | Used for |
| :--- | :--- |
| **2911** | The KivaDB protocol: shells and client programs ([Chapter 18](#chapter-18--the-tcp-server)) |
| **8080** | The REST API over HTTP ([Chapter 19](#chapter-19--the-rest-api)) |
| **6379** | The Redis protocol ([Chapter 31](#chapter-31--redis-compatibility)) |

By default both ports only accept connections from the same computer (`127.0.0.1`). Opening KivaDB to other machines is explained in [section 15.7](#157-opening-kivadb-to-the-network).

What the server prints goes to `server.log` in the home directory.

### 2.2 Status

```bash
kivadb status
```

```text
KivaDB is running (pid 21040).
  TCP port : 2911
  Home     : C:\Users\you\AppData\Roaming\KivaDB
```

or, when it is stopped:

```text
KivaDB is not running. Start it with: kivadb start
```

### 2.3 Stop

```bash
kivadb stop
```

```text
KivaDB is shutting down.
KivaDB stopped.
```

Stopping is clean: everything is flushed to disk first. (Even an unclean stop loses nothing, see [Chapter 16](#chapter-16--maintenance-and-reliability).) You can also type `shutdown` in a shell; it does the same thing and requires the admin role.

### 2.4 Options of `start`

| Option | Meaning |
| :--- | :--- |
| `--port <n>` | KivaDB protocol port (default 2911) |
| `--http-port <n>` | REST API port (default 8080) |
| `--no-http` | Do not start the REST API |
| `--resp-port <n>` | Redis-compatible port (default 6379) |
| `--no-resp` | Do not start the Redis-compatible listener |
| `--bind <address>` | Address to listen on (default 127.0.0.1) |
| `--home <folder>` | Use another home directory |

The same values can be set permanently in the configuration file ([Chapter 14](#chapter-14--configuration)).

### 2.5 Running in the foreground

`kivadb serve` runs the same server but stays attached to the terminal and prints its log there. Stop it with Ctrl+C. Use it when you want to watch what happens, or when another program (systemd, Docker, a service manager) is in charge of keeping KivaDB alive.

```bash
kivadb serve
```

```text
KivaDB 1.0.0 server
  Home         : C:\Users\you\AppData\Roaming\KivaDB
  TCP protocol : 127.0.0.1:2911 (binary and plain text)
  REST API     : http://127.0.0.1:8080  (docs: /docs, metrics: /metrics, events: /ws)
  Auth         : disabled (local use)
  Replication  : master
Press Ctrl+C to stop.
```

### 2.6 Starting KivaDB with the computer

KivaDB does not install itself as a system service; use what your system provides. The short versions are below; [Chapter 29](#chapter-29--deploying-kivadb-on-a-server) gives complete, step-by-step instructions for Linux, Windows, macOS and Docker.

**Windows** - a scheduled task that runs at logon:

```bash
schtasks /Create /TN "KivaDB" /TR "\"C:\Tools\kivadb.exe\" start" /SC ONLOGON
```

**Linux** - a systemd unit, `/etc/systemd/system/kivadb.service`:

```ini
[Unit]
Description=KivaDB
After=network.target

[Service]
User=kivadb
Environment=KIVADB_HOME=/var/lib/kivadb
ExecStart=/usr/local/bin/kivadb serve
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now kivadb
```

### 2.7 What if I open a shell without starting KivaDB?

It works. If nothing is running, the `kivadb` shell opens the data itself, and **while that shell is open** the database is reachable on port 2911 like a started one (on another free port if 2911 is taken). When you close the shell, the database stops. Use `kivadb start` when the data must stay available.

Only one process at a time opens the files; every other `kivadb` you launch connects to it. That is why two terminals always see the same data.

---

## Chapter 3 — First steps in the shell

### 3.1 Open the shell

```bash
kivadb
```

```text
KivaDB Shell v1.0.0
Type 'help' or 'h' for command list
kiva>
```

`kiva>` is the **prompt**: KivaDB is waiting for a command. If the database was started with `kivadb start`, a line such as `Connected to KivaDB 1.0.0 at 127.0.0.1:2911` is shown first.

### 3.2 Store and read a value

```text
kiva> set name "Fordi"
OK: name saved.
(0.000412s)
kiva> get name
name: Fordi [string]
(0.000108s)
```

* `set name "Fordi"` stores the text `Fordi` under the key `name`.
* `get name` reads it back. The answer shows the key, the value, and its type in brackets.
* The last line is the time the command took. It is left out of the examples from here on.

### 3.3 Several things at once

The word `and` repeats a command:

```text
kiva> set age 30 and city "Kinshasa" and active true
OK: age saved.
OK: city saved.
OK: active saved.
kiva> get name and age and ghost
name: Fordi [string]
age: 30 [number]
ghost: (nil)
```

`(nil)` means "there is no such key".

### 3.4 Change and delete

`set` only creates. To change an existing key, use `update`:

```text
kiva> set name "Other"
Error: Key 'name' already exists. Use 'update' to change it.
kiva> update age 31
OK: age updated.
kiva> del city
Deleted: city
```

This separation protects you from overwriting a value by mistake.

### 3.5 See what is stored

```text
kiva> scan

--- KivaDB Scan (workspace: default) ---
 KEY                      | TYPE    |       SIZE | MODIFIED            | STATUS
------------------------------------------------------------------------------
 active                   | boolean |    4 bytes | 2026-10-12 09:44:54 |
 age                      | number  |    2 bytes | 2026-10-12 09:44:54 |
 name                     | string  |    5 bytes | 2026-10-12 09:44:54 |
------------------------------------------------------------------------------
Total: 3 keys.
```

### 3.6 Editing, history and completion

In a terminal the shell behaves like a modern command line:

| Key | Effect |
| :--- | :--- |
| Left / Right, Home / End | Move in the line |
| Up / Down | Previous and next commands |
| Tab | Complete a command or the next keyword (`get sim` + Tab gives `get similar`) |
| Ctrl+R | Search the history |
| Ctrl+C | Abandon the line being typed |
| Ctrl+D | Leave the shell |

The history is kept between sessions in `history.txt` in the home directory. Commands that contain a password (`create user ... identified by`, `modify user ... password`, `auth`) are never written to it.

### 3.7 Help and exit

| Command | Effect |
| :--- | :--- |
| `help` or `h` | Lists every command |
| `clear` | Clears the screen |
| `exit` or `quit` | Leaves the shell (the database keeps running if it was started) |

### 3.8 Running commands without entering the shell

`-e` runs one command and exits. It can be repeated. This is how you use KivaDB from scripts and batch files:

```bash
kivadb -e "set visits 1" -e "get visits"
```

```text
OK: visits saved.
visits: 1 [number]
```

The exit code is `0` when every command succeeded and `1` when one of them reported an error.

A file of commands can be fed on standard input; lines starting with `#` are comments:

```bash
kivadb < commands.txt
```

---

## Chapter 4 — Syntax rules

A few rules apply to every command. They exist so that KivaDB never has to guess what you meant.

### 4.1 Keys

A key is written as plain text, without quotes.

| Rule | Example |
| :--- | :--- |
| Letters, digits and most symbols are allowed | `age`, `user:1`, `price_eur`, `log-2026.10` |
| A key cannot be made of digits only | `44` is refused, `id44` is fine |
| No spaces, quotes, commas, brackets, parentheses, `*` or `?` | |
| At most 1024 bytes | |
| Keys are case-sensitive | `Name` and `name` are two keys |

A common convention is to group keys with a prefix and a colon: `user:1`, `user:2`, `order:2026:001`. KivaDB keeps keys sorted, so keys with the same prefix sit next to each other, which makes them easy to list and query.

```text
kiva> set 44 1
Error: InvalidKeyName: '44' cannot be purely numeric.
kiva> set "name" 1
Error: Key 'name' cannot use quotes. Use bare text or backticks (``).
```

**Reserved words.** Words that are part of the language (`set`, `get`, `and`, `to`, `string`, `limit`, ...) cannot be used as keys directly. Put them between backticks if you really need one:

```text
kiva> set get 1
Error: 'get' is a reserved keyword and cannot be used as a key.
kiva> set `get` 1
OK: get saved.
kiva> get `get`
get: 1 [number]
```

### 4.2 Values

How you write a value tells KivaDB its type:

| You write | Type | Notes |
| :--- | :--- | :--- |
| `"Hello"` or `'Hello'` | string | Text **must** be quoted |
| `42`, `-3.14`, `1e9` | number | Numbers **must not** be quoted |
| `true`, `false` | boolean | Not quoted |
| `null` | null | "Exists, but has no value yet" |
| `["a", "b"]` | array | A JSON list |
| `{"name": "Fordi"}` | object | A JSON document |
| `0xDEADBEEF`, `b64:SGVsbG8=` | bytes | Raw data in hexadecimal or base64 |

```text
kiva> set city Kinshasa
Error: String values like 'Kinshasa' must be quoted ("" or '').
```

There are no escape sequences inside strings. To put a double quote in a text, surround the text with single quotes, and the other way round:

```text
kiva> set quote 'He said "hello"'
OK: quote saved.
```

### 4.3 Chaining with `and`

`and` separates several operations of the same command. Each one succeeds or fails on its own:

```text
kiva> set x 1 and y oops and z 3
OK: x saved.
Error: String values like 'oops' must be quoted ("" or '').
OK: z saved.
```

Chaining works with `set`, `get`, `update`, `del`, `has` and `typeof`.

### 4.4 Type prefixes

Most data commands accept an optional type before the key. It turns the command into a check: "do this, but only if the type is what I expect".

```text
kiva> set number age 30
OK: age saved.
kiva> get string age
Error: Type mismatch. 'age' is a [number].
kiva> del string age
Error: Type mismatch. Cannot delete 'age' because it is a [number].
```

### 4.5 Upper and lower case

Command words can be written in any case (`BEGIN`, `begin`). Keys and values are kept exactly as typed.

### 4.6 Comments

A line that starts with `#` or `--` is ignored. Useful in command files.

---

## Chapter 5 — Data types

KivaDB has seven types. `typeof` tells you the type of a key:

```text
kiva> typeof age
 -> age is a [number]
```

### 5.1 string

Text, in UTF-8. Always quoted.

```text
kiva> set greeting "Mbote na yo"
OK: greeting saved.
```

### 5.2 number

Integers and decimals, optionally with an exponent. KivaDB stores the number exactly as you wrote it.

```text
kiva> set price 19.99 and big 1e9 and debt -250
OK: price saved.
OK: big saved.
OK: debt saved.
```

### 5.3 boolean

`true` or `false`.

### 5.4 null

A key can exist without having a value yet. That is different from a key that does not exist:

```text
kiva> set pending null
OK: pending saved.
kiva> get pending and ghost
pending: null [null]
ghost: (nil)
kiva> has pending and has ghost
Yes: Key 'pending' exists.
No: Key 'ghost' does not exist.
```

A `null` key has no type yet, so its first `update` may give it a value of any type:

```text
kiva> update pending 12
OK: pending updated.
kiva> get pending
pending: 12 [number]
```

### 5.5 array

An ordered list, written as JSON. Elements can be of any type, including other arrays and objects.

```text
kiva> set colors ["red", "green", "blue"]
OK: colors saved.
kiva> get colors and colors[0]
colors: ["red","green","blue"] [array]
colors[0]: red [string]
```

Positions start at 0. `colors.0` means the same as `colors[0]`.

### 5.6 object

A document: named fields, written as JSON. Field names are quoted, as JSON requires.

```text
kiva> set user:1 {"name": "Alice", "role": "admin", "age": 30, "address": {"city": "Kinshasa"}}
OK: user:1 saved.
kiva> get user:1.address.city
user:1.address.city: Kinshasa [string]
```

Arrays and objects are covered in depth in [Chapter 10](#chapter-10--documents).

### 5.7 bytes

Raw binary data: a small image, a certificate, anything that is not text.

```text
kiva> set logo 0x89504E47
OK: logo saved.
kiva> set word b64:SGVsbG8=
OK: word saved.
kiva> set bytes note "Hello"
OK: note saved.
kiva> get logo and word and note
logo: 0x89504e47 [bytes]
word: 0x48656c6c6f [bytes]
note: 0x48656c6c6f [bytes]
```

Three ways to write bytes: hexadecimal after `0x`, base64 after `b64:`, or the `bytes` prefix followed by a quoted text. The shell always displays bytes in hexadecimal (only the first 64 bytes of large values).

### 5.8 Types are enforced

`update` never changes the type of a key:

```text
kiva> update age "thirty"
Error: Cannot update [number] with a value formatted as [string].
```

When you really want to change the type, say so explicitly with `change` ([section 6.6](#66-change--rename-or-migrate)).

---

## Chapter 6 — Writing, reading and deleting

### 6.1 `set` — create a key

```text
set [type] <key> <value> [ttl <seconds>]
```

Creates a new key. Fails if the key already exists.

```text
kiva> set login_retry 0
OK: login_retry saved.
kiva> set string motto "Kitoko"
OK: motto saved.
kiva> set session "abc123" ttl 3600
OK: session saved.
```

`ttl` makes the key disappear by itself after that many seconds ([Chapter 7](#chapter-7--expiring-keys-ttl)).

### 6.2 `get` — read

```text
get [type] <key>
```

```text
kiva> get motto
motto: Kitoko [string]
kiva> get number login_retry
login_retry: 0 [number]
kiva> get ghost
ghost: (nil)
```

A key followed by a path reads one part of a document: `get user:1.name`, `get colors[1]`.

### 6.3 `update` — replace the value

```text
update [type] <key> <value>
```

Replaces the value of an existing key. The new value must have the same type. If the key has a TTL, it keeps it.

```text
kiva> update login_retry 5
OK: login_retry updated.
kiva> update ghost 1
Error: Key 'ghost' not found. Use 'set' to create it.
```

### 6.4 `has` and `typeof` — ask without reading

Both answer from memory, without touching the disk.

```text
kiva> has motto
Yes: Key 'motto' exists.
kiva> has number motto
No: Key 'motto' exists but type is [string] (expected [number]).
kiva> typeof motto and ghost
 -> motto is a [string]
 -> ghost does not exist (nil)
```

### 6.5 `del` — delete

```text
del [type] <key>
del all keys
```

```text
kiva> del motto and ghost
Deleted: motto
Not found: ghost
```

`del all keys` empties the current workspace completely. It needs the admin role and cannot be undone:

```text
kiva> del all keys
All keys deleted. Database reset.
```

### 6.6 `change` — rename or migrate

```text
change [type] <old> to [type] <new> [value]
```

`change` does two jobs.

**Rename** a key (the value, type and TTL are kept):

```text
kiva> set active_id 7
OK: active_id saved.
kiva> change active_id to session_id
Renamed: active_id -> session_id (Type preserved)
```

**Migrate** a key to a new value, possibly of another type:

```text
kiva> set legacy_code "old"
OK: legacy_code saved.
kiva> change string legacy_code to number system_code 1044
Migrated: legacy_code -> system_code (Type: number)
```

Both forms are atomic: the old key disappears only if the new one was written. A failed migration leaves everything as it was:

```text
kiva> change system_code to boolean flag maybe
Error: Migration failed ('maybe' is not a valid boolean (use true or false).) Source key 'system_code' has been preserved.
```

`change` refuses to overwrite an existing key.

---

## Chapter 7 — Expiring keys (TTL)

TTL means "time to live". A key with a TTL deletes itself when the time is up. Typical uses: login sessions, temporary codes, cached results.

### 7.1 Setting a TTL

```text
kiva> set session "abc123" ttl 3600
OK: session saved.
```

`session` will exist for 3600 seconds (one hour). `scan` shows the remaining time in the STATUS column: `TTL: 3600s`.

### 7.2 `bump` — extend or reset

```text
bump [type] <key> add|set <seconds>
```

* `add` adds time to what is left.
* `set` restarts the countdown from now. This is the classic "sliding session": every time the user does something, give them a fresh hour.

```text
kiva> bump session add 600
OK: Key 'session' TTL updated successfully (add 600s).
kiva> bump session set 3600
OK: Key 'session' TTL updated successfully (set 3600s).
kiva> bump ghost set 10
Error: Key 'ghost' not found or already expired.
```

`bump` also works on a key that had no TTL: it gives it one.

### 7.3 What happens at expiry

Once the time has passed, the key behaves exactly as if it had been deleted: `get` answers `(nil)`, `has` answers no, and it no longer appears in listings. `update` keeps the TTL of a key; `change` keeps it when renaming.

---

## Chapter 8 — Calculations and text with print

`print` evaluates expressions and displays the result. It does not store anything.

### 8.1 Arithmetic

Operators: `+`, `-`, `*`, `/`, `%` (remainder), with parentheses.

```text
kiva> print (4 + 8) * 10
120
kiva> print 7 / 2, 7 % 2
3.5 1
```

### 8.2 Using stored values

Write a key name and `print` uses its value:

```text
kiva> set age 31
OK: age saved.
kiva> print (age + 9) * 2, "years"
80 years
```

Several expressions separated by commas are displayed on one line, separated by spaces. Paths into documents work too: `print user:1.age + 1`.

### 8.3 Text and interpolation

Inside a quoted text, `${...}` is replaced by the result of the expression between the braces:

```text
kiva> set name "Fordi"
OK: name saved.
kiva> print "Hello ${name}, next year you will be ${age + 1}"
Hello Fordi, next year you will be 32
```

`+` joins two texts:

```text
kiva> print "Kiva" + "DB"
KivaDB
```

### 8.4 Errors

`print` never guesses:

```text
kiva> print "a" + 1
Error: TypeError: cannot concatenate str and int
kiva> print 1 / 0
Error: ZeroDivisionError: division by zero
kiva> print ghost
Error: NameError: 'ghost' is not defined
kiva> print 1 2
Error: SyntaxError: missing separator between tokens.
```

A key whose name contains a character that looks like an operator (`-` for example) must be written between backticks in an expression: ``print `log-count` + 1``.

---

## Chapter 9 — Listing keys and ranges

KivaDB keeps all keys **sorted** in memory. Listing a slice of them is therefore very cheap. The examples below use these keys:

```text
kiva> set user:1 {"name": "Alice", "role": "admin", "age": 30, "address": {"city": "Kinshasa"}}
kiva> set user:2 {"name": "Bob", "role": "dev", "age": 25, "address": {"city": "Goma"}}
kiva> set user:3 {"name": "Carol", "role": "dev", "age": 41, "address": {"city": "Kinshasa"}}
kiva> set active true and age 31 and colors ["red", "green", "blue"]
```

### 9.1 `keys` — names only

```text
keys [like <pattern>] [from <A> to <B>] [limit N] [reverse]
```

```text
kiva> keys
active
age
colors
user:1
user:2
user:3
(6 keys)
kiva> keys like "user:*" limit 2
user:1
user:2
(2 keys)
kiva> keys from "b" to "v" reverse
user:3
user:2
user:1
colors
(4 keys)
```

In a pattern, `*` stands for any run of characters and `?` for exactly one character.

### 9.2 `scan` — names with details

```text
scan [pattern] [limit N]
```

Shows, for each key: type, size, last modification and status (remaining TTL, compression, MessagePack encoding).

```text
kiva> scan "user:*"

--- KivaDB Scan (workspace: default) ---
 KEY                      | TYPE    |       SIZE | MODIFIED            | STATUS
------------------------------------------------------------------------------
 user:1                   | object  |   70 bytes | 2026-10-12 09:44:54 |
 user:2                   | object  |   62 bytes | 2026-10-12 09:44:54 |
 user:3                   | object  |   68 bytes | 2026-10-12 09:44:54 |
------------------------------------------------------------------------------
Total: 3 keys.
```

### 9.3 `range` — keys and values between two bounds

```text
range <from> to <to> [reverse] [limit N] [return field1, field2]
```

Both bounds are included. `get range ...` is accepted as a synonym.

```text
kiva> range "user:1" to "user:2"
user:1: {"name":"Alice","role":"admin","age":30,"address":{"city":"Kinshasa"}} [object]
user:2: {"name":"Bob","role":"dev","age":25,"address":{"city":"Goma"}} [object]
(2 keys)
```

`return` keeps only some fields of each document (a **projection**):

```text
kiva> range "user:1" to "user:3" return name, role
user:1: {"name":"Alice","role":"admin"}
user:2: {"name":"Bob","role":"dev"}
user:3: {"name":"Carol","role":"dev"}
(3 keys)
```

`reverse` walks from the highest key down, and `limit` stops after N results. Together they answer "the last N":

```text
kiva> range "user:1" to "user:9" reverse limit 1 return name
user:3: {"name":"Carol"}
(1 keys)
```

### 9.4 Why ranges matter

Because keys are sorted as text, well-chosen key names give you useful queries for free:

* **Pagination**: `keys from "user:" to "user:~" limit 20`, then start again from the last key you received.
* **Time slices**: with keys like `log:2026-10-12T09:44`, `range "log:2026-10-12T09" to "log:2026-10-12T10"` returns one hour of logs.
* **Groups**: everything starting with `sensor:` is `keys like "sensor:*"`.

Keys are compared character by character, so numbers inside keys should be padded with zeros: `order:007` sorts before `order:010`, but `order:7` sorts after `order:10`.

---

## Chapter 10 — Documents

Arrays and objects are stored as real structures, not as text. You can read and modify a single field without rewriting the rest yourself.

### 10.1 Paths

A **path** designates a place inside a document:

| Path | Meaning |
| :--- | :--- |
| `.name` | the field `name` |
| `.address.city` | the field `city` inside the field `address` |
| `.tags[0]` or `.tags.0` | the first element of the array `tags` |

### 10.2 Reading a field

Directly with `get`:

```text
kiva> get user:1.name and user:1.address.city
user:1.name: Alice [string]
user:1.address.city: Kinshasa [string]
kiva> get user:1.missing
user:1.missing: (nil)
```

Or with `json get`, which takes the key and the path separately:

```text
kiva> json get user:1 .address
user:1.address: {"city":"Kinshasa"} [object]
```

If a key's name itself contains a dot (`config.v2`) and exists, `get config.v2` reads that key; the path interpretation is only tried when no such key exists.

### 10.3 Changing a field

```text
json set <key> <.path> <value>
```

```text
kiva> json set user:2 .role "lead"
OK: user:2.role updated.
kiva> json set user:2 .address.zip 243
OK: user:2.address.zip updated.
kiva> get user:2
user:2: {"name":"Bob","role":"lead","age":25,"address":{"city":"Goma","zip":243}} [object]
```

Missing intermediate fields are created. In an array, you can replace an existing position or append at the position just after the last one.

### 10.4 Removing a field

```text
kiva> json delete user:2 .age
Deleted: user:2.age
kiva> json delete user:2 .nothing
Not found: user:2.nothing
```

### 10.5 Replacing the whole document

`update` with a new JSON literal replaces everything:

```text
kiva> update colors ["black", "white"]
OK: colors updated.
```

### 10.6 JSON text stored as a string

If you store JSON inside quotes, it is a string, not a document:

```text
kiva> set raw '{"name": "Fordi"}'
OK: raw saved.
kiva> typeof raw
 -> raw is a [string]
```

Reading with a path still works on such strings (`get raw.name`), but `json set`, indexes and `where` queries need a real document. To store quoted JSON as a document, add the type:

```text
kiva> set object parsed '{"name": "Fordi"}'
OK: parsed saved.
```

### 10.7 MessagePack encoding

Documents are written to disk as JSON text by default. MessagePack is a binary encoding of the same data: smaller and faster to decode. You choose per key, and nothing else changes; every command behaves the same.

```text
kiva> convert user:1 to msgpack
OK: user:1 converted to MessagePack (70 -> 51 bytes).
kiva> get user:1.name
user:1.name: Alice [string]
kiva> convert user:1 to json
OK: user:1 converted to JSON (51 -> 70 bytes).
```

`scan` shows `msgpack` in the STATUS column for keys stored that way.

---

## Chapter 11 — Queries and indexes

So far every command named the keys it wanted. A **query** finds documents by what they contain. The examples of this chapter use the three users exactly as they were created at the start of [Chapter 9](#chapter-9--listing-keys-and-ranges).

### 11.1 The condition

```text
<pattern>.<field> <operator> <value>
```

* `pattern` selects keys (`user:*`, or `*` for all).
* `field` is a path inside the document (`role`, `address.city`).
* `operator` is one of `=`, `!=`, `>`, `>=`, `<`, `<=`.
* `value` follows the usual rules: text quoted, numbers and booleans bare.

The field starts at the first dot after the last `*`: in `user:*.address.city`, the pattern is `user:*` and the field is `address.city`. Spaces around the operator are optional (`user:*.age>26`). If the pattern itself needs quoting, quote the whole left side: `"*.active" = true`.

### 11.2 `get where` — find documents

```text
get where <condition> [limit N] [return field1, field2]
```

```text
kiva> get where user:*.address.city = "Kinshasa"
user:1: {"name":"Alice","role":"admin","age":30,"address":{"city":"Kinshasa"}}
user:3: {"name":"Carol","role":"dev","age":41,"address":{"city":"Kinshasa"}}
(2 matches)
kiva> get where user:*.age < 35 return name, address.city
user:1: {"name":"Alice","address.city":"Kinshasa"}
user:2: {"name":"Bob","address.city":"Goma"}
(2 matches)
```

Comparisons are made between values of the same kind: numbers with numbers, text with text (alphabetical order). A document that lacks the field never matches.

### 11.3 `count` and `sum` — aggregates

```text
kiva> count where user:*.age > 26
Count: 2
kiva> sum where user:*.age > 0
Sum: 96 (3 matches)
```

`sum` adds up the field named in the condition, over the documents that satisfy it.

### 11.4 Indexes — making queries fast

Without help, a query has to open every document that matches the pattern. A **secondary index** keeps, in memory, the list of keys for each value of a field, so the answer is found directly.

```text
create index on <pattern> (<field>)
```

```text
kiva> create index on user: (role)
Index created on user:* (role): 3 documents indexed.
```

A pattern without wildcard is taken as a prefix: `user:` means `user:*`.

From then on the index is kept up to date by every write, automatically. Index definitions are saved and the indexes are rebuilt when KivaDB starts.

```text
kiva> indexes
user:* (role) - 3 documents indexed
kiva> drop index on user: (role)
Index on user:* (role) dropped.
```

Create an index on fields you filter on often. Each index uses memory and makes writes on matching keys slightly slower, so do not index everything.

### 11.5 `explain` — see how a query will run

```text
kiva> explain get where user:*.role = "dev"
Query Plan
  Filter    : user:*.role = "dev"
  Strategy  : PREFIX SCAN
  Index     : none
  Estimated : 3 of 8 keys examined
  Tip       : create index on user:* (role)
```

After creating the index:

```text
kiva> explain get where user:*.role = "dev"
Query Plan
  Filter    : user:*.role = "dev"
  Strategy  : INDEX SCAN
  Index     : user:* (role)
  Estimated : 2 of 8 keys examined
```

| Strategy | Meaning |
| :--- | :--- |
| `INDEX SCAN` | A secondary index gives the candidates directly |
| `PREFIX SCAN` | Only the keys sharing the pattern's prefix are examined |
| `FULL SCAN` | Every key is examined (pattern starting with `*`) |
| `ORDERED INDEX RANGE SCAN` | A `range` query: a slice of the sorted keys |
| `POINT LOOKUP` | A `get` on one key |

`explain` also works with `count where`, `sum where`, `range` and `get <key>`. An index is used for `=`, `>`, `>=`, `<` and `<=`; the `!=` operator always scans.

---

## Chapter 12 — Transactions

### 12.1 The problem

Imagine moving money: take 50 from one account, add 50 to another. If the program stops between the two steps, 50 has vanished. A **transaction** groups changes so that they all happen, or none of them does.

### 12.2 begin, commit, rollback

```text
kiva> set account:a 100 and account:b 100
OK: account:a saved.
OK: account:b saved.
kiva> begin
Transaction 2 started.
kiva> update account:a 50
OK: account:a updated.
kiva> update account:b 150
OK: account:b updated.
kiva> commit
Transaction committed (2 keys written).
```

Between `begin` and `commit`, nothing is written to the database yet: the changes are kept aside. Other users do not see them. `commit` applies them all in one step; `rollback` throws them away:

```text
kiva> begin
Transaction 3 started.
kiva> update account:a 0
OK: account:a updated.
kiva> rollback
Transaction rolled back.
kiva> get account:a
account:a: 50 [number]
```

Inside your transaction, `get`, `has`, `typeof`, `json` and `print` see your pending changes. Listing and query commands (`keys`, `scan`, `range`, `where`) show what is committed.

### 12.3 Savepoints

A **savepoint** is a bookmark inside a transaction. You can come back to it without abandoning what was done before it.

```text
kiva> begin
Transaction 4 started.
kiva> set order:1 {"total": 50}
OK: order:1 saved.
kiva> savepoint before_delete
Savepoint 'before_delete' created.
kiva> del colors
Deleted: colors
kiva> rollback to before_delete
Rolled back to savepoint 'before_delete'.
kiva> commit
Transaction committed (1 keys written).
```

`order:1` was written; the deletion of `colors` was cancelled.

### 12.4 Status

```text
kiva> transaction status
Transaction ID: 4
State: ACTIVE
Savepoints: 1
Modified keys: 2
```

Outside a transaction it answers `State: NONE (no active transaction)`.

### 12.5 Errors inside a transaction

If a data command fails inside a transaction, the whole transaction is cancelled. You never end up with half of your changes applied by accident:

```text
kiva> begin
Transaction 5 started.
kiva> update account:a 1
OK: account:a updated.
kiva> update account:a "text"
Error: Cannot update [number] with a value formatted as [string].
Transaction rolled back (an error occurred inside the transaction).
```

### 12.6 Conflicts between users

Two people may open a transaction at the same time and touch the same key. KivaDB lets both work, and checks at `commit`: if a key of your transaction was modified by someone else since your `begin`, your commit is refused and nothing of your transaction is applied.

```text
Terminal A                          Terminal B
----------                          ----------
begin                               begin
update balance 10                   update balance 20
commit        -> committed
                                    commit
                                    Error: ERR_TRANSACTION_CONFLICT: key 'balance'
                                    was modified by another session
```

When that happens, start again: `begin`, read the fresh values, redo the changes, `commit`. This approach is called **optimistic** concurrency: no one waits for anyone, and conflicts, which are rare, are detected at the end.

### 12.7 Guarantees

* **Atomic on disk.** A committed transaction is written as one batch whose last record marks the commit. If the machine stops in the middle, the incomplete batch is ignored at the next start.
* **Leaving cancels.** Closing the shell or losing the connection with a transaction open is a rollback.
* `del all keys` and `workspace switch` are refused inside a transaction.

---

## Chapter 13 — Workspaces

A **workspace** is an independent database: its own keys, its own files, its own indexes. Use one per project, or one for tests and one for real data.

### 13.1 The default workspace

There is always a workspace named `default`. It is where an administrator lands.

### 13.2 Creating and switching

```text
kiva> workspace create shop
Workspace 'shop' created at C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop.
kiva> workspace switch shop
Switched to workspace 'shop' (0 keys).
kiva> set product:1 {"name": "Pen", "price": 2}
OK: product:1 saved.
kiva> workspace list
   default              write  C:\Users\you\AppData\Roaming\KivaDB\data
 * shop                 write  C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop
kiva> workspace switch default
Switched to workspace 'default' (11 keys).
```

The `*` marks the workspace you are in. `workspace current` shows only that one.

To keep a workspace somewhere specific (another disk, a project folder), give the path:

```text
kiva> workspace create projet_a "D:\projects\a\db"
```

### 13.3 Opening a workspace directly

```bash
kivadb -w shop
```

### 13.4 Removing

```text
kiva> workspace remove shop
Workspace 'shop' unregistered. Its data files were left on disk.
```

KivaDB forgets the workspace but does not delete its files; remove the folder yourself if you want the data gone. The `default` workspace and the one you are currently in cannot be removed.

### 13.5 Things to know

* Workspace names may contain letters, digits, `_` and `-`.
* Creating and removing workspaces requires the admin role.
* Each non-admin user also has a personal workspace ([section 15.4](#154-workspaces-and-grants)).
* Stored scripts, triggers and users are shared by all workspaces. Indexes belong to one workspace.

---

## Chapter 14 — Configuration

### 14.1 The file

All settings live in `kiva.conf`, in the home directory. It is created with default values on the first run. You can edit it with a text editor (while KivaDB is stopped) or, more conveniently, with the `config` command.

```ini
[storage]
default_workspace = C:\Users\you\AppData\Roaming\KivaDB\data
auto_compact = true
compact_threshold = 30
compression = none
sync_writes = false
snapshot_interval = 300
snapshot_every = 1000
cache_values = false
cache_size_mb = 256

[workspaces]
shop = C:\Users\you\AppData\Roaming\KivaDB\workspaces\shop

[network]
port = 2911
bind = 127.0.0.1

[security]
auth_required = false

[server]
threads = 8
max_clients = 1000

[http]
enabled = true
port = 8080
rate_limit = 600
cors = true
stream = true

[resp]
enabled = true
port = 6379

[replication]
role = master
master_host =
master_port = 2911
master_user = kiva
master_password =
sync_mode = async
min_slaves_ack = 1

[scripting]
timeout_ms = 5000
memory_limit_mb = 64
```

### 14.2 The `config` command

```text
kiva> config get compression
storage.compression = none
kiva> config set compact_threshold 25
OK: storage.compact_threshold = 25
kiva> config list
Configuration file: C:\Users\you\AppData\Roaming\KivaDB\kiva.conf
  storage.default_workspace        = C:\Users\you\AppData\Roaming\KivaDB\data
  storage.auto_compact             = true
  ...
```

When a name exists in several sections, write `section.key`:

```text
kiva> config get port
Error: 'port' exists in several sections; use one of: network.port, http.port
kiva> config get network.port
network.port = 2911
```

`config` requires the admin role. Values are checked (`config set compact_threshold abc` is refused). Passwords are shown as `***`.

### 14.3 Every setting

| Setting | Default | Meaning | Takes effect |
| :--- | :--- | :--- | :--- |
| `storage.default_workspace` | `<home>/data` | Folder of the `default` workspace | next start |
| `storage.auto_compact` | `true` | Compact automatically ([16.3](#163-compaction)) | immediately |
| `storage.compact_threshold` | `30` | Fragmentation percentage that triggers it | immediately |
| `storage.compression` | `none` | `none`, `lz4` or `zlib` for new writes ([16.5](#165-compression)) | immediately |
| `storage.sync_writes` | `false` | Force every write to the physical disk ([16.2](#162-what-survives-what)) | next start |
| `storage.snapshot_interval` | `300` | Seconds between index snapshots | next start |
| `storage.snapshot_every` | `1000` | ...or after this many modifications | next start |
| `storage.cache_values` | `false` | Keep recently read values in memory ([Chapter 32](#chapter-32--keeping-values-in-memory-value-cache)) | immediately |
| `storage.cache_size_mb` | `256` | Memory budget of that cache, per workspace | immediately |
| `network.port` | `2911` | KivaDB protocol port | next start |
| `network.bind` | `127.0.0.1` | Address to listen on | next start |
| `security.auth_required` | `false` | Ask for a user name and password | immediately |
| `server.threads` | `8` | Commands executed at the same time | next start |
| `server.max_clients` | `1000` | Simultaneous connections | immediately |
| `http.enabled` | `true` | Start the REST API | next start |
| `http.port` | `8080` | REST API port | next start |
| `http.rate_limit` | `600` | Requests per minute per client address, `0` = unlimited | immediately |
| `http.cors` | `true` | Allow browsers on other sites to call the API | immediately |
| `http.stream` | `true` | Offer `/v1/stream` (Server-Sent Events) | immediately |
| `resp.enabled` | `true` | Start the Redis-compatible listener ([Chapter 31](#chapter-31--redis-compatibility)) | next start |
| `resp.port` | `6379` | Its port | next start |
| `replication.*` | | See [Chapter 22](#chapter-22--replication) | |
| `scripting.timeout_ms` | `5000` | Longest a Lua script may run | immediately |
| `scripting.memory_limit_mb` | `64` | Most memory a Lua script may use | immediately |

---

## Chapter 15 — Users, roles and security

### 15.1 The starting point

A fresh KivaDB has one account, `kiva`, which is an administrator with an **empty password**, and authentication is **off**: anyone who can run `kivadb` on the machine is `kiva`. That is convenient on your own computer. Before sharing the machine or opening the network, secure it ([section 15.6](#156-turning-authentication-on)).

```text
kiva> whoami
kiva (admin) @ workspace 'default'
```

### 15.2 Roles

Every user has one role:

| Role | Can do |
| :--- | :--- |
| `admin` | Everything, in every workspace |
| `editor` | Read and write data, transactions, run scripts |
| `viewer` | Read only: `get`, `has`, `typeof`, `print`, listings, queries |

Administration commands (`compact`, `compress`, `config`, `create`, `drop`, `grant`, `revoke`, `users`, `audit`, `replica`, `del all keys`, `shutdown`, ...) need `admin`.

### 15.3 Managing accounts

```text
kiva> create user "fordi" identified by "mon_pass"
User 'fordi' created with role [editor].
kiva> create user "guest" identified by "guest_pass" role viewer
User 'guest' created with role [viewer].
kiva> users
 USER                 | ROLE    | CREATED             | WORKSPACE GRANTS
------------------------------------------------------------------------------
 fordi                | editor  | 2026-10-12 09:44:55 | u_fordi=write
 guest                | viewer  | 2026-10-12 09:44:55 | u_guest=write
 kiva                 | admin   | 2026-10-12 09:44:53 | all (admin)
```

A new user is an `editor` unless you say otherwise. User names may contain letters, digits, `_`, `-` and `.`.

Changing an account:

```text
kiva> modify user "fordi" password "new_pass"
Password updated for 'fordi'.
kiva> modify user "guest" role "editor"
Role of 'guest' set to [editor].
kiva> drop user "guest"
User 'guest' dropped.
```

Anyone may change their own password; only an admin may change someone else's password or a role. The `kiva` account cannot be dropped or demoted.

### 15.4 Workspaces and grants

* An **admin** can use every workspace.
* Every other user has a **personal workspace**, named `u_<user name>`, created automatically. That is where they arrive after logging in, and nobody else (except admins) can enter it.
* To let a user into another workspace, an admin **grants** access:

```text
kiva> grant read on workspace "shop" to "guest"
Granted: read on workspace 'shop' to 'guest'.
kiva> grant write on all to "fordi"
Granted: write on all workspaces to 'fordi'.
```

| Command | Effect |
| :--- | :--- |
| `grant read on workspace "<w>" to "<u>"` | The user may read that workspace |
| `grant write on workspace "<w>" to "<u>"` | The user may read and write it |
| `grant read\|write on all to "<u>"` | Same, for every workspace |
| `grant admin on all to "<u>"` | The user becomes an admin |
| `revoke write on workspace "<w>" from "<u>"` | Write access becomes read access |
| `revoke read on workspace "<w>" from "<u>"` | Access is removed |
| `revoke write from "<u>"` | The user can no longer write anywhere (becomes a viewer) |
| `revoke read from "<u>"` | Every grant is removed |
| `revoke admin from "<u>"` | The user becomes an editor |

What a user can actually do in a workspace is the lower of their role and their access to it: an `editor` with *read* access can only read there; a `viewer` with *write* access still cannot write. Granting *write* to a viewer turns them into an editor.

A user enters a workspace they were granted with `workspace switch <name>`. Changes to grants apply the next time the user switches workspace or logs in.

### 15.5 Logging in

When authentication is on, the shell asks who you are:

```text
$ kivadb
Username [kiva]: fordi
Password: ********
Welcome fordi (editor)
kiva> whoami
fordi (editor) @ workspace 'u_fordi'
```

Pressing Enter at the user name prompt chooses `kiva`, the built-in administrator. On the command line: `kivadb -u fordi` asks for the password; `kivadb -u fordi -p mon_pass` gives it directly (convenient in scripts, but the password is then visible in your command history).

Inside any session, `auth <user> <password>` logs in or switches user. This is also how you log in over a plain-text connection ([Chapter 18](#chapter-18--the-tcp-server)).

### 15.6 Turning authentication on

Do it in this order, so you do not lock yourself out:

```text
kiva> modify user "kiva" password "a-long-and-secret-password"
Password updated for 'kiva'.
kiva> config set auth_required true
OK: security.auth_required = true
```

From now on every new shell, connection and HTTP request must present a user name and password. As long as an account has an empty password, KivaDB reminds you at login:

```text
[WARN] kiva password is empty. Please run: modify user "kiva" password "..."
```

### 15.7 Opening KivaDB to the network

By default KivaDB only listens on `127.0.0.1`, which means only programs on the same computer can connect. To accept connections from other machines:

1. Turn authentication on and give `kiva` a password ([15.6](#156-turning-authentication-on)).
2. `config set bind 0.0.0.0` (all network interfaces) or the address of one interface.
3. Restart KivaDB (`kivadb stop`, `kivadb start`).

KivaDB **refuses to start** on a non-local address while authentication is off or `kiva` has no password:

```text
Error: refusing to listen on 0.0.0.0 without authentication.
```

Two more protections are always active: an account with an empty password can only log in from the local machine, and the REST API limits the number of requests per client ([19.7](#197-cors-and-rate-limiting)).

**Encryption.** KivaDB 1.0.0 does not encrypt its connections. On a network you do not fully control, put it behind something that does: a TLS-terminating reverse proxy (nginx, Caddy) for the REST API, a VPN or an SSH tunnel for port 2911.

### 15.8 How passwords are stored

Never in clear. For each account KivaDB keeps a random *salt* and the result of PBKDF2-HMAC-SHA256 (60 000 rounds) applied to the password. Accounts are stored in KivaDB's own engine, in a system database inside the home directory (`system/system.kiva`).

### 15.9 The audit log

Sensitive actions are recorded in `audit.log` in the home directory: account and privilege changes, deletions, compaction, configuration changes, logins. Passwords are masked.

```text
kiva> audit last 3
[2026-10-12 09:44:55] user=kiva action="create user 'fordi' identified by '***'" status=SUCCESS
[2026-10-12 09:44:55] user=kiva action="create user 'guest' identified by '***' role viewer" status=SUCCESS
[2026-10-12 09:44:55] user=kiva action="grant read on workspace 'shop' to 'guest'" status=SUCCESS
```

| Command | Shows |
| :--- | :--- |
| `audit last <N>` | The N most recent entries |
| `audit since "2026-10-01"` | Entries from that date (or `"2026-10-01 14:00:00"`) |
| `audit failed-logins` | Rejected login attempts |

### 15.10 Emergency access

If you lose the kiva password, someone with access to the machine can start KivaDB without authentication, fix the account, and restart normally:

```bash
kivadb stop
kivadb --skip-auth
```

```text
[WARN] Authentication disabled - unsafe for production
kiva> modify user "kiva" password "new-password"
```

---

## Chapter 16 — Maintenance and reliability

### 16.1 `stats` — the dashboard

```text
kiva> stats

==========================================================
     KivaDB Engine Statistics
----------------------------------------------------------
 > Workspace            : default
 > Total Keys           : 11 (1 with TTL)
 > Storage Size         : 1.12 KB
 > Live Data Size       : 639 bytes
 > Disk Fragmentation   : 44.4% [Action Needed: 'compact']
 > Memory Usage (RAM)   : 1.82 KB
 > Compression          : none (254 bytes -> 254 bytes, 0.0% saved, 0 records)
 > Startup              : 4.12 ms (new database)
 > Checksum Errors      : 0
 > Last Integrity Check : never
 > Auto-Compaction      : on (threshold 30%), 0 run(s)
 > Secondary Indexes    : 1 (+ 0 vector)
 > Value Cache          : off (values are read from disk)
 > Replication          : master, 0 replica(s), max lag 0 ops
 > Database Path        : C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva
==========================================================
```

| Line | Meaning |
| :--- | :--- |
| Storage Size | Size of the data file on disk |
| Live Data Size | The part of it that holds current values |
| Disk Fragmentation | The share that is dead weight (old versions, deleted keys) |
| Memory Usage | Estimated RAM used by the index |
| Compression | Size of values before and after compression |
| Startup | How long loading took, and how (`new database`, `from hint file`, `full log scan`) |
| Checksum Errors | Damaged records detected since start |
| Last Integrity Check | When `check` last ran and what it found |
| Value Cache | Memory used by cached values and the share of reads it answered ([Chapter 32](#chapter-32--keeping-values-in-memory-value-cache)) |

### 16.2 What survives what

| Event | Result |
| :--- | :--- |
| The shell or server is closed normally | Nothing is lost |
| The process crashes or is killed | Nothing is lost. A write that was cut in half is discarded at the next start |
| Power failure or operating system crash | By default, up to the last second of writes can be lost. With `sync_writes = true`, nothing is |
| A transaction was open | It is as if it never started |
| A compaction was running | The original file is intact; the unfinished copy is thrown away |

`sync_writes = true` asks the disk to confirm every single write. It is the safest setting and makes writes noticeably slower; the default (confirm once per second) is the usual compromise.

After an unclean stop, the shell tells you what it did:

```text
[INFO] Recovered after an unclean shutdown: 37 bytes of incomplete log discarded.
```

### 16.3 Compaction

KivaDB never modifies data in place. An `update` appends the new value at the end of the file; a `del` appends a marker saying the key is gone. This is fast and safe, but the old versions stay in the file. **Compaction** rewrites the file with only what is current.

```text
kiva> compact
Database storage compacted successfully (1.12 KB -> 639 bytes, 11 records).
```

Compaction does not stop the database: reads and writes continue while the bulk of the copy happens.

**Automatic compaction** does this for you in the background when fragmentation reaches a threshold (30 % by default) on a file of at least 64 KB:

```text
kiva> autocompact status
Auto-compaction: ON
  Threshold: 30% fragmentation
  Current fragmentation: 0.0%
  Compactions run: 2
kiva> autocompact threshold 25
Auto-compaction is ON (threshold 25%).
kiva> autocompact off
Auto-compaction is OFF (threshold 25%).
```

### 16.4 Integrity: `check`

Every record on disk carries a checksum (CRC32). Whenever a value is read, the checksum is verified; a damaged record is never returned as if it were valid:

```text
kiva> get victim
Error: ERR_DATA_CORRUPTED: checksum mismatch on key 'victim'
```

`check` verifies everything at once:

```text
kiva> check
Checking 11 entries...
  OK: 11
  CORRUPTED: 0
Done.
```

If some records are damaged (failing disk, file modified by another program), `check repair` saves a copy of the data file next to it (`store.kiva.bak`) and removes the unreadable entries:

```text
kiva> check repair
Checking 3 entries...
  OK: 2
  CORRUPTED: 1
    - victim
  Backup saved to C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva.bak
  Repairing... 1 entries removed.
Done.
```

A removed entry cannot be recovered by KivaDB; restore it from your own backup or from a replica.

### 16.5 Compression

Values of 64 bytes or more can be compressed before being written.

| Algorithm | Character |
| :--- | :--- |
| `lz4` | Very fast, good ratio. The usual choice |
| `zlib` | Smaller files, slower |
| `none` | No compression |

```text
kiva> compress lz4
Database re-encoded with compression 'lz4': 12.40 MB -> 3.10 MB on disk (50000 records, values 76.2% smaller).
```

(The figures above are an illustration; yours depend on your data.)

```text
kiva> config get compression
storage.compression = lz4
```

`compress` does two things: it rewrites the whole current workspace with the chosen algorithm, and it makes that algorithm the setting for all future writes (`storage.compression`). Reading is transparent: `get` returns the original value whatever the compression. A file can contain records compressed in different ways; `scan` shows the algorithm of each key in the STATUS column. Values that would not shrink are stored as they are.

### 16.6 The hint file

To answer quickly, KivaDB keeps in memory an **index**: for each key, where its value is in the data file. Rebuilding that index by reading the whole data file would be slow for a large database, so KivaDB regularly saves a compact copy of the index in a **hint file**. At start it loads the hint file, then reads only what was written after it.

```text
kiva> hint info
Hint File: C:\Users\you\AppData\Roaming\KivaDB\data\store.kiva.hint
  Status: VALID
  Entries: 11
  Size on disk: 545 bytes
  Compression: none
  Last updated: 2026-10-12 09:44:54
  Checksum: PASSED
```

The hint file is written at every clean stop, after every compaction, and periodically (`snapshot_interval`, `snapshot_every`). It is protected by checksums; if it is missing or damaged, KivaDB simply reads the data file in full. Deleting it is harmless.

### 16.7 Backups

A workspace is a folder. To back it up:

1. Stop KivaDB (`kivadb stop`) or, at least, make sure nothing is writing.
2. Copy the workspace folder (the important file is `store.kiva`; the others can be rebuilt).

To back up everything (users, configuration, all default-location workspaces), copy the whole home directory. For a copy that stays up to date continuously, use a replica ([Chapter 22](#chapter-22--replication)).

### 16.8 Measuring speed

```text
kiva> bench write 100000
WRITE: 100000 ops in 3.514s -> 28459 ops/sec (avg 0.035 ms)
kiva> bench read 100000
READ : 100000 ops in 0.973s -> 102739 ops/sec (avg 0.010 ms)
```

`bench` writes temporary keys (`__bench:...`) in the current workspace and removes them afterwards. The figures depend on your disk and processor.

---

## Chapter 17 — How data is stored

You do not need this chapter to use KivaDB. It is here for the curious, and for anyone who has to understand the files.

### 17.1 The home directory

```text
KivaDB/
  kiva.conf            configuration
  audit.log            audit journal
  server.log           output of 'kivadb start'
  instance.lock        held by the running KivaDB process
  instance.port        its TCP port (removed when it stops)
  instance.pid         its process id (removed when it stops)
  system/
    system.kiva        users, stored scripts, triggers
  data/                the 'default' workspace
    store.kiva
    store.kiva.hint
    store.kiva.meta
    store.kiva.lock
  workspaces/
    shop/              another workspace
      store.kiva
      ...
```

### 17.2 The files of a workspace

| File | Role | If deleted |
| :--- | :--- | :--- |
| `store.kiva` | The data | The data is lost |
| `store.kiva.hint` | Index snapshot for fast start | Rebuilt; the next start is slower |
| `store.kiva.meta` | Definitions of secondary indexes | Indexes must be created again |
| `store.kiva.lock` | Prevents two processes from opening the files | Recreated |
| `store.kiva.bak` | Copy made before a repair | - |

### 17.3 The principle: an append-only log

`store.kiva` is a **log**: a sequence of records, each one added after the previous. Nothing already written is ever changed.

```text
[header][record: set a=1][record: set b=2][record: set a=5][record: delete b] ...
```

To know the current state, read the log from the start and keep, for each key, the last thing said about it. KivaDB does that once, at start, and keeps the result in memory as a sorted index (key → position of its latest record). After that:

* a **read** is one lookup in memory and one read on disk;
* a **write** is one append at the end of the file and one update in memory.

This design is simple, fast for writes, and safe: a crash can only damage the very last record, and that is detected.

### 17.4 The data file, byte by byte

All integers are little-endian.

**File header (16 bytes)**

| Field | Size | Content |
| :--- | :--- | :--- |
| magic | 4 | The letters `KIVA` |
| format | 4 | Storage format number: `1` |
| file_id | 8 | A random number, changed at every compaction and reset |

**Record (28-byte header, then the key, then the value)**

| Field | Type | Size | Content |
| :--- | :--- | :--- | :--- |
| crc32 | `u32` | 4 | Checksum of all the following bytes of the record |
| k_size | `u16` | 2 | Length of the key |
| v_size | `u32` | 4 | Length of the value as stored |
| type | `u8` | 1 | 1 string, 2 number, 3 boolean, 4 null, 5 array, 6 object, 7 bytes |
| flags | `u8` | 1 | See below |
| timestamp | `i64` | 8 | When it was written (seconds since 1970) |
| expires_at | `i64` | 8 | When it expires; 0 = never |
| key | bytes | k_size | The key, UTF-8 |
| value | bytes | v_size | The value |

**Flags**

| Bits | Meaning |
| :--- | :--- |
| 0-1 | Compression: 0 none, 1 LZ4, 2 Zlib |
| 2 | The document is encoded in MessagePack instead of JSON |
| 5 | The record belongs to a batch (transaction) |
| 6 | The record is the last of its batch: the commit point |
| 7 | Tombstone: the key was deleted |

**How values are encoded**

| Type | Bytes |
| :--- | :--- |
| string | The UTF-8 text |
| number | The number as text (`3.14`) |
| boolean | `true` or `false` |
| null | Nothing (length 0) |
| array, object | JSON text, or MessagePack if flag bit 2 is set |
| bytes | The raw bytes |

A compressed value starts with its original length (`u32`), followed by the compressed data.

**Transactions.** The records of a transaction all carry the batch flag, and the last one also carries the commit flag. When reading the log, KivaDB sets batch records aside and applies them only when it meets the commit record. A batch without its commit record, which can only be at the very end of the file after a crash, is discarded.

### 17.5 The hint file

Header (44 bytes): the letters `KHNT`, a version number, the `file_id` of the data file it belongs to, the length of the data file it covers, the number of entries, the compression used, a checksum of the body and a checksum of the header.

Body: one entry per key - key, position in the data file, sizes, type, flags, timestamps - each followed by its own CRC32. The body is compressed with the workspace's algorithm.

The `file_id` is what makes the hint safe: if the data file was replaced (compaction, reset), the ids no longer match and the hint is ignored.

### 17.6 Limits

| Item | Limit |
| :--- | :--- |
| Key length | 1024 bytes |
| Value size | 256 MB |
| Number of keys | Bounded by memory: every key and about 100 bytes of bookkeeping live in RAM |
| Data file size | Bounded by the disk |

Values are **not** kept in memory, only keys and positions (unless the value cache of [Chapter 32](#chapter-32--keeping-values-in-memory-value-cache) is turned on, within its budget). A database can therefore be much larger than the RAM, as long as the keys fit.

---

## Chapter 18 — The TCP server

When KivaDB is running, it listens on a TCP port, **2911** by default. That one port understands two languages.

### 18.1 Plain text

Send a command, a line break, and read the answer. It is exactly the shell language. Any tool that opens a TCP connection can use it:

```bash
telnet localhost 2911
```

```text
set user:9 "Alice"
OK: user:9 saved.
get user:9
user:9: Alice [string]
exit
```

With authentication on, start with `auth`:

```text
get user:9
Error: permission denied: authentication required. Use: auth <user> <password>
auth fordi mon_pass
Welcome fordi (editor)
```

Text mode is ideal for trying things and for simple scripts. Its weakness: an answer can have any number of lines and nothing marks its end, so a program cannot reliably tell where one answer stops. Programs should use the binary protocol or the REST API.

### 18.2 The binary protocol

Every message, in both directions, is a **frame**:

```text
+----------------+-------------+------------------+
| length (u32)   | code (u16)  | payload          |
+----------------+-------------+------------------+
```

* Integers are big-endian (network order).
* `length` counts the code and the payload, that is, payload size + 2.
* In a request, `code` is a command number; in a reply, a status.

KivaDB decides which language a connection speaks from its very first byte: `0x00` means binary (the first byte of a small frame length is always zero), anything else means text.

**Requests**

| Command | Code | Payload | Reply payload |
| :--- | :--- | :--- | :--- |
| HELLO | 1 | empty | Three lines: `KivaDB 1.0.0`, `auth=required` or `auth=none`, `user=<name>` |
| AUTH | 2 | user, a zero byte, password | A welcome text |
| EXEC | 3 | A command line, as in the shell | Output lines separated by `\n`; each line starts with `+` (normal) or `-` (error) |
| PING | 4 | empty | `PONG` |
| GET | 5 | The key | One type byte, then the value bytes (see [17.4](#174-the-data-file-byte-by-byte); documents are JSON text) |
| SET | 6 | type `u8`, ttl `u32`, key length `u16`, key, value bytes | `OK` |
| DEL | 7 | The key | `1` |
| API | 8 | A JSON object with an `op` field | A JSON object (see [18.4](#184-structured-requests-for-tools)) |

SET creates the key or replaces it, whatever its previous type. A ttl of 0 means no expiry.

**Reply statuses**

| Status | Meaning |
| :--- | :--- |
| 0 | Success |
| 1 | Error; the payload is the message (for EXEC: at least one line starts with `-`) |
| 2 | Key not found (GET, DEL) |
| 3 | The server is closing the connection (after `exit` or `shutdown`) |
| 4 | The command was `clear` |
| 9 | A change notification ([Chapter 20](#chapter-20--change-notifications)); not a reply to a request |

**Example.** The request `EXEC get age` is these 13 bytes:

```text
00 00 00 09    length = 9 (2 for the code + 7 for the payload)
00 03          code = 3 (EXEC)
67 65 74 20 61 67 65    "get age"
```

and the reply:

```text
00 00 00 13    length = 19 (2 for the status + 17 for the payload)
00 00          status = 0
2B 61 67 65 3A 20 39 39 20 5B 6E 75 6D 62 65 72 5D    "+age: 99 [number]"
```

A connection stays open for as many requests as you like. Replies come back in the order of the requests. Ready-made clients are in [Chapter 21](#chapter-21--connecting-from-your-programs).

### 18.3 Capacity

Each connection is served by its own thread. `server.max_clients` (1000) caps the number of connections; `server.threads` (8) caps how many commands execute at the same instant.

### 18.4 Structured requests for tools

The command language answers with text written for people. A program that needs data it can rely on - a graphical client, a dashboard - uses the API request (code 8): a JSON object goes in, a JSON object comes back. The same session, permissions and transaction apply as for commands.

| `op` | Other fields | Answer |
| :--- | :--- | :--- |
| `whoami` | - | `user`, `role`, `workspace`, `access`, `can_write`, `auth_required`, `replica`, `version` |
| `workspaces` | - | `workspaces`: name, path, access and whether it is the current one |
| `scan` | `pattern` (default `*`), `limit` (default 500) | `keys`: key, type, size, modified, ttl, compression, msgpack; `truncated`; `total` |
| `get` | `key` | `found`, `type`, `value`, `ttl`, `modified`, `msgpack` |
| `put` | `key`, `type`, `value`, optional `ttl`, optional `mode` | `key`, `type` |
| `del` | `key` | `deleted` |
| `stats` | - | The figures of the `stats` command, as fields |
| `users` | - | `users`: name, role, grants, created, home workspace, whether the password is empty (admin only) |
| `query` | `target` (`user:*.age`), `cmp` (`=` `!=` `>` `>=` `<` `<=`), `value`, `limit` | `rows`: key and document; `plan` |
| `explain` | the same as `query` | `plan`: strategy, index, candidates, total keys; `suggestion`: the index to create, if any |
| `similar` | `target` (`doc:*.embedding`), `vector`, `limit`, `metric` | `results`: key, score, document; `compared`; `indexed` |

Notes on `put`:

* `value` is plain JSON matching `type`. A number may be given as a JSON number or as text (`"3.14"`), bytes are base64 text.
* `mode` is `create` (fail if the key exists), `update` (fail if it does not) or `upsert` (default).
* `ttl` absent keeps the current expiry of the key, `0` removes it, a positive number sets it in seconds.

Because values travel as JSON, they are free of the shell's quoting rules: a text may contain both kinds of quotes.

Request and answer for reading a key:

```json
{"op": "get", "key": "user:1"}
```

```json
{"found": true, "key": "user:1", "type": "object", "value": {"name": "Alice"}, "ttl": null, "modified": 1791791094, "msgpack": false}
```

An error comes back with status 1 and the message as payload, as for every request. The Rust client exposes this as `Client::api`. Over HTTP, send the same JSON to `POST /v1/api`.

---

## Chapter 19 — The REST API

The REST API lets anything that speaks HTTP use KivaDB: a browser, `curl`, any programming language. It listens on port **8080**.

### 19.1 The shape of answers

Every answer is JSON.

```json
{"status": "ok", "data": "...", "timestamp": 1791791113}
```

```json
{"status": "error", "error": "key 'ghost' not found", "timestamp": 1791791113}
```

| HTTP code | Meaning |
| :--- | :--- |
| 200 | Success |
| 400 | The request is malformed or the value is invalid |
| 401 | Credentials are missing or wrong |
| 403 | You are not allowed to do this |
| 404 | The key or the route does not exist |
| 409 | Conflict (for example: the node is a read-only replica) |
| 429 | Too many requests |

### 19.2 Keys

**Write** - `POST /v1/key/{key}` creates the key or replaces it. The body is the value, in JSON:

```bash
curl -X POST http://localhost:8080/v1/key/user:1 -d '{"name": "Alice", "role": "admin"}'
```

```json
{"key":"user:1","type":"object","status":"ok","timestamp":1791791113}
```

To set an expiry, wrap the value: `{"value": ..., "ttl": seconds}` (or add `?ttl=60` to the URL).

```bash
curl -X POST http://localhost:8080/v1/key/session:42 -d '{"value": "abc123", "ttl": 3600}'
```

The JSON type decides the KivaDB type: a JSON string becomes a string, a number a number, and so on. A body that is not valid JSON is stored as a string.

**Read** - `GET /v1/key/{key}`:

```bash
curl http://localhost:8080/v1/key/user:1
```

```json
{"key":"user:1","type":"object","data":{"name":"Alice","role":"admin"},"ttl":null,"status":"ok","timestamp":1791791113}
```

`ttl` is the number of seconds left, or `null`. Add `?path=.name` to read one field of a document. Values of type bytes are returned in base64.

**Delete** - `DELETE /v1/key/{key}`:

```bash
curl -X DELETE http://localhost:8080/v1/key/user:1
```

```json
{"key":"user:1","deleted":true,"status":"ok","timestamp":1791791113}
```

### 19.3 Listings

| Route | Parameters | Returns |
| :--- | :--- | :--- |
| `GET /v1/keys` | `from`, `to`, `limit`, `reverse=true` | Key names, in order |
| `GET /v1/scan` | `pattern`, `limit` | Keys with type, size, modification time, ttl |
| `GET /v1/range` | `from`, `to`, `limit`, `reverse=true` | Keys with their values |

```bash
curl "http://localhost:8080/v1/keys?from=user:&to=user:9&limit=2"
```

```json
{"count":2,"data":["user:1","user:2"],"status":"ok","timestamp":1791791113}
```

### 19.4 Any command

`POST /v1/exec` runs a shell command and returns what it printed. Everything in this manual is therefore available over HTTP.

```bash
curl -X POST http://localhost:8080/v1/exec -d '{"command": "count where user:*.age > 26"}'
```

```json
{"ok":true,"output":["Count: 2"],"status":"ok","timestamp":1791791113}
```

`ok` is `false` when the command reported an error. Each HTTP request is independent: a transaction cannot span several requests.

### 19.5 Other routes

| Route | Purpose |
| :--- | :--- |
| `GET /v1/stats` | Engine statistics as JSON |
| `GET /metrics` | Prometheus metrics ([Chapter 24](#chapter-24--monitoring)) |
| `GET /ws` | WebSocket change notifications ([Chapter 20](#chapter-20--change-notifications)) |
| `GET /v1/stream` | Server-Sent Events change notifications ([20.4](#204-over-server-sent-events)) |
| `POST /v1/api` | Structured requests, including `query` and `similar` ([18.4](#184-structured-requests-for-tools)) |
| `GET /health` | `{"version":"1.0.0","status":"ok","timestamp":...}` - no login needed |
| `GET /docs` | An interactive page listing the routes, with a "Try it" button for each |
| `GET /openapi.json` | The same description in OpenAPI format |

### 19.6 Authentication and workspaces

With `auth_required`, send the user name and password with HTTP Basic authentication:

```bash
curl -u fordi:mon_pass http://localhost:8080/v1/key/user:1
```

Without valid credentials the answer is `401`. Requests are executed in the user's home workspace; add `?workspace=<name>` to work in another one you have access to:

```bash
curl -u fordi:mon_pass "http://localhost:8080/v1/key/product:1?workspace=shop"
```

### 19.7 CORS and rate limiting

* **CORS.** By default the API sends the headers that allow a web page served from another site to call it (`Access-Control-Allow-Origin: *`). Turn it off with `config set cors false`.
* **Rate limiting.** Each client address may send `http.rate_limit` requests per minute (600 by default). Beyond that the answer is `429` with a `Retry-After` header. Set it to `0` to disable.

The REST API speaks plain HTTP. For HTTPS, put a reverse proxy in front of it.

---

## Chapter 20 — Change notifications

Instead of asking again and again whether something changed, a client can **subscribe** and be told.

### 20.1 In the shell

```text
kiva> subscribe user:*
Subscribed to 'user:*' in workspace 'default'. Changes will be announced as they happen.
```

When anyone - another shell, a program, an HTTP request - creates, changes or deletes a matching key, a line appears:

```text
[event] set user:7 (object)
[event] update user:7 (object)
[event] del user:7
```

| Event | Meaning |
| :--- | :--- |
| `set <key> (<type>)` | The key was created |
| `update <key> (<type>)` | An existing key was overwritten |
| `del <key>` | The key was deleted |
| `reset (all keys deleted)` | `del all keys` was run |

`unsubscribe` stops everything; `unsubscribe user:*` stops one pattern. Subscriptions belong to the workspace you were in when you subscribed and end when you leave it or disconnect.

### 20.2 Over TCP

On a text connection, notifications arrive as lines starting with `[event] `. On a binary connection they arrive as frames with status 9, at any moment, including between a request and its reply; a client must set them aside and keep waiting for its reply.

### 20.3 Over WebSocket

Web pages and applications can use the WebSocket endpoint of the REST port:

```javascript
const ws = new WebSocket("ws://localhost:8080/ws?pattern=user:*");
ws.onmessage = (message) => console.log(JSON.parse(message.data));
```

The first message confirms the subscription; then one message per change:

```json
{"event":"subscribed","pattern":"user:*","workspace":"default"}
{"event":"set","key":"user:5","type":"object","timestamp":1791791113}
{"event":"del","key":"user:5","type":null,"timestamp":1791791114}
```

Without `pattern`, every key is watched. With authentication on, the WebSocket request must carry the same Basic credentials as any other request.

### 20.4 Over Server-Sent Events

Server-Sent Events (SSE) is plain HTTP that stays open and receives messages, one way. It is simpler than a WebSocket, passes through proxies easily, and browsers support it natively with `EventSource`.

```bash
curl -N "http://localhost:8080/v1/stream?pattern=order:*"
```

```text
retry: 2000

event: subscribed
data: {"event":"subscribed","pattern":"order:*","workspace":"default"}

id: 1
event: set
data: {"event":"set","key":"order:7","type":"object","timestamp":1791791113}
```

In a web page:

```javascript
const stream = new EventSource("http://localhost:8080/v1/stream?pattern=order:*");
stream.addEventListener("set", (e) => console.log(JSON.parse(e.data)));
stream.addEventListener("del", (e) => console.log(JSON.parse(e.data)));
```

A comment line is sent every 15 seconds so that idle connections stay open. With authentication on, send Basic credentials (from a browser, a reverse proxy usually adds them). `config set stream false` turns the endpoint off.

---

## Chapter 21 — Connecting from your programs

Start KivaDB (`kivadb start`), then pick the approach that suits your language.

| Approach | Good for | Needs |
| :--- | :--- | :--- |
| REST API (port 8080) | Any language, web applications, quick integration | An HTTP library |
| Binary protocol (port 2911) | Lower overhead, long-lived connections, notifications | A few lines of socket code, or a ready client |
| Command line (`kivadb -e`) | Shell scripts, batch files | Nothing |

### 21.1 Command line

```bash
kivadb -e "set backup_done true" -e "get backup_done"
```

Check the exit code: `0` success, `1` at least one error.

### 21.2 curl

See [Chapter 19](#chapter-19--the-rest-api). Summary:

```bash
curl -X POST http://localhost:8080/v1/key/city:1 -d '{"name": "Kinshasa"}'
curl http://localhost:8080/v1/key/city:1
curl -X DELETE http://localhost:8080/v1/key/city:1
```

### 21.3 Node.js

**With the REST API** (Node 18 or later, nothing to install):

```javascript
async function main() {
  const base = "http://127.0.0.1:8080";
  await fetch(`${base}/v1/key/city:1`, {
    method: "POST",
    body: JSON.stringify({ value: { name: "Kinshasa" }, ttl: 3600 }),
  });
  const reply = await (await fetch(`${base}/v1/key/city:1`)).json();
  console.log(reply.status, reply.type, reply.data.name);   // ok object Kinshasa
}
main();
```

**With the official driver.** The `kivadb` package in [`drivers/node`](drivers/node) speaks the binary protocol, has no dependencies and ships TypeScript types. It is ready to publish on npm; until then, install it from the folder (`npm install path/to/kiva-db/drivers/node`):

```javascript
const { connect } = require("kivadb");

const db = await connect({ host: "localhost", port: 2911, user: "kiva", password: "" });
await db.set("user:1", { name: "Alice", role: "admin" }, { ttl: 3600 });
console.log(await db.get("user:1"));
console.log(await db.similar("doc:*.embedding", [0.1, 0.2, 0.3], { limit: 5 }));
db.on("change", (e) => console.log(e.event, e.key));
await db.subscribe("user:*");
```

Its README lists every method. **A minimal client to read**: [`examples/nodejs/kiva.js`](examples/nodejs/kiva.js) shows the protocol in about fifty lines:

```javascript
const { KivaClient } = require("./kiva");

async function main() {
  const db = new KivaClient("127.0.0.1", 2911);
  db.onEvent = (event) => console.log("event:", event);
  // await db.auth("fordi", "mon_pass");      // when authentication is on

  await db.exec('set product:1 {"name": "Pen", "price": 2}');
  console.log(await db.exec("get product:1.name"));
  console.log(await db.exec("count where product:*.price < 10"));

  await db.exec("subscribe product:*");
  await db.exec("json set product:1 .price 3");
  try {
    await db.exec('update product:1 "not an object"');
  } catch (err) {
    console.log("refused:", err.message);
  }
  await db.exec("del product:1");
  setTimeout(() => db.close(), 200);
}

main();
```

Output:

```text
[ 'product:1.name: Pen [string]' ]
[ 'Count: 1' ]
event: update product:1 (object)
refused: Error: Cannot update [object] with a value formatted as [string].
event: del product:1
```

`exec` sends any command of this manual and returns the output lines; it throws when the command reports an error.

### 21.4 Python

With the REST API and the standard library only:

```python
import json
import urllib.request

BASE = "http://127.0.0.1:8080"

def call(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    request = urllib.request.Request(BASE + path, data=data, method=method)
    with urllib.request.urlopen(request) as response:
        return json.loads(response.read())

call("POST", "/v1/key/city:1", {"value": {"name": "Kinshasa"}, "ttl": 3600})
print(call("GET", "/v1/key/city:1")["data"]["name"])          # Kinshasa
print(call("POST", "/v1/exec", {"command": "keys limit 5"})["output"])
call("DELETE", "/v1/key/city:1")
```

When authentication is on, add the header `Authorization: Basic <base64 of "user:password">` to each request. Note that `urlopen` raises an exception for answers such as 404.

### 21.5 Rust

Add KivaDB as a dependency of your project and use its client:

```rust
use kivadb::client::Client;
use kivadb::Value;

fn main() -> Result<(), kivadb::KivaError> {
    let mut db = Client::connect("127.0.0.1:2911")?;
    // db.auth("fordi", "mon_pass")?;        // when authentication is on

    db.set("greeting", &Value::Str("hello".into()), 0)?;
    db.set("session", &Value::Str("abc123".into()), 3600)?;   // expires in an hour
    assert_eq!(db.get("greeting")?, Some(Value::Str("hello".into())));
    assert_eq!(db.get("ghost")?, None);

    println!("{}", db.exec("count where user:*.role = \"admin\"")?);
    db.del("greeting")?;
    Ok(())
}
```

| Method | Does |
| :--- | :--- |
| `Client::connect(addr)` | Opens a connection |
| `auth(user, password)` | Logs in |
| `get(key)` | Returns `Option<Value>` |
| `set(key, &value, ttl_secs)` | Creates or replaces; ttl 0 = no expiry |
| `del(key)` | Returns whether the key existed |
| `exec(command)` | Runs any command; returns its output, or an error |
| `ping()` | Checks the connection |
| `events` | Notifications received so far, after a `subscribe` |

### 21.6 Redis libraries

Any Redis client library works for the commands of [Chapter 31](#chapter-31--redis-compatibility): point it at port 6379. This is often the fastest way to use KivaDB from a language that has no KivaDB driver yet (Python's `redis`, Go's `go-redis`, PHP's `phpredis`, Java's `Jedis`...).

### 21.7 Kiva Studio, the graphical client

Kiva Studio is the desktop application for KivaDB (Windows, Linux, macOS). It lives next to the database in the repository, in the `kiva-studio` folder, and connects to port 2911 like any other client: enter the host, the port, a user name and a password. It offers a data browser and editor (with a tree view for documents), a visual query builder that shows the execution plan and creates the missing index in one click, vector similarity search, a command console, an activity feed with alerts (fragmentation, damaged data, replication, failed logins), user management for administrators, a health dashboard, and one-click switching between remembered servers. See its own README for how to build and run it.

### 21.8 Any other language

Two options: use the REST API with your language's HTTP library, or implement the binary protocol of [section 18.2](#182-the-binary-protocol) (with the structured requests of [section 18.4](#184-structured-requests-for-tools) if you want JSON instead of text). The protocol is small; the Node.js client is about fifty lines and is a good model: build a frame, send it, read frames until one is not a notification.

---

## Chapter 22 — Replication

### 22.1 What it is

Replication keeps a live copy of a database on another KivaDB. The one that receives the writes is the **master**; the copies are **replicas**. Uses:

* **Safety**: if the master's disk dies, a replica has the data.
* **Read load**: applications can read from replicas.
* **Standby**: a replica can be promoted to master.

Replication covers the `default` workspace.

### 22.2 Setting it up

You need two running KivaDB, usually on two machines. On the future master, nothing special is required beyond being reachable from the replica ([15.7](#157-opening-kivadb-to-the-network)).

On the replica, tell it whom to follow:

```text
kiva> replica of 192.168.1.10 2911
Now replicating from 192.168.1.10:2911. Local data is replaced by the master's.
```

What happens then:

1. The replica **erases its own default workspace**.
2. The master sends a full copy (a snapshot).
3. The master then sends every change as it happens.

The choice is remembered in the configuration: after a restart, the replica resumes following its master. If the connection drops, the replica retries every two seconds and resynchronises by itself.

If the master requires authentication, give the replica an admin account to log in with:

```text
kiva> config set master_user "kiva"
kiva> config set master_password "the-master-password"
```

### 22.3 A replica is read-only

```text
kiva> set nope 1
Error: READONLY: this node is a replica; write to the master or run 'replica promote'
```

### 22.4 Watching it

On the master:

```text
kiva> replica status
Role: master
  Mode: async (min acks: 1)
  Sequence: 1024
  Connected replicas: 1
    #7 192.168.1.20:51344 (listens on 192.168.1.20:2911) lag: 0 ops, connected 2026-10-12 09:50:01
```

*Lag* is how many operations the replica has not confirmed yet; 0 means it is up to date.

On the replica:

```text
kiva> replica status
Role: replica (read-only)
  Master: 192.168.1.10:2911
  Link: connected
  Applied sequence: 1024 (318 operations since connect)
  Last sync: 2026-10-12 09:50:01
```

### 22.5 Asynchronous or synchronous

* **Asynchronous** (default). The master answers as soon as it has written locally and sends the change to replicas right after. Fastest; if the master dies at the wrong instant, the very last writes may not have reached a replica.
* **Synchronous.** The master waits until `min_slaves_ack` replicas have confirmed each write before answering.

```text
kiva> config set sync_mode sync
kiva> config set min_slaves_ack 1
```

If the confirmations do not arrive within five seconds, the write is kept and a warning is shown:

```text
Warning: write acknowledged by 0 of 1 required replica(s).
```

In asynchronous mode you can still wait at chosen moments with `wait`:

```text
kiva> set important "data"
OK: important saved.
kiva> wait 1
OK: 1 replica(s) acknowledged.
```

`wait <N> [timeout_ms]` blocks until N replicas have everything written so far (default timeout: 5000 ms).

### 22.6 Commands

| Command | Where | Effect |
| :--- | :--- | :--- |
| `replica of <host> <port>` | replica | Start following that master |
| `replica of none` | replica | Stop and become a master |
| `replica stop` | replica | Pause following (stays read-only) |
| `replica start` | replica | Follow the configured master again |
| `replica sync` | replica | Force a full resynchronisation |
| `replica promote` | replica | Become a writable master |
| `replica status` | both | Show the state |
| `wait <N> [timeout_ms]` | master | Wait for N replicas to be up to date |
| `failover to <replica>` | master | Hand over the master role |

### 22.7 Failover

**Planned** - the master is alive and you want to switch roles (maintenance, migration). On the master, name the replica by its number from `replica status`:

```text
kiva> failover to 7
Failover complete: 192.168.1.20:2911 is the new master and this node now follows it.
Other replicas must be pointed at the new master with: replica of <host> <port>
```

The master waits for the replica to be up to date, tells it to become master, and becomes its replica.

**Unplanned** - the master is dead. On the replica you choose as the new master:

```text
kiva> replica promote
This node is now a master.
```

Then point your applications, and the other replicas, to it. This step is manual: KivaDB 1.0.0 does not elect a new master by itself.

---

## Chapter 23 — Lua scripting

For logic that needs several steps - read a value, decide, write another - you can run small programs **inside** KivaDB, written in [Lua](https://www.lua.org) 5.4. The steps run next to the data, without network round trips, and atomically.

### 23.1 `eval` — run code

```text
eval '<lua code>' [arguments...]
```

```text
kiva> eval 'return kiva.get("user:1").name'
Alice
kiva> eval 'return 6 * 7'
42
```

The code goes between single quotes so that double quotes can be used inside. What the script `return`s is displayed. A single expression is displayed even without `return` (that is why `kiva.set(...)` alone shows `true`). A script that returns nothing displays `OK`.

### 23.2 The `kiva` functions

| Function | Does |
| :--- | :--- |
| `kiva.get(key)` | Returns the value, or `nil` if the key does not exist |
| `kiva.set(key, value [, ttl])` | Creates or replaces the key; `ttl` in seconds |
| `kiva.del(key)` | Deletes; returns `true` if the key existed |
| `kiva.has(key)` | `true` or `false` |
| `kiva.typeof(key)` | The type name, or `nil` |
| `kiva.scan([pattern])` | A list of key names (all keys without pattern) |
| `kiva.incr(key [, amount])` | Adds to a number (1 by default), creating it at 0 if needed; returns the new value |

How values travel between KivaDB and Lua:

| KivaDB | Lua |
| :--- | :--- |
| string | string |
| number | number |
| boolean | boolean |
| null | `nil` |
| array | table with positions 1, 2, 3... |
| object | table with named fields |
| bytes | string |

```text
kiva> eval 'kiva.set("visits", (kiva.get("visits") or 0) + 1) return kiva.get("visits")'
1
kiva> eval 'kiva.set("made", {list = {1, 2, 3}, ok = true})'
true
kiva> get made.list
made.list: [1,2,3] [array]
```

Lua tables count from 1: `kiva.get("colors")[1]` is the first colour. An empty Lua table is stored as an empty object.

### 23.3 Arguments and print

Words after the code are available in the table `ARGV`, as text. `print` shows lines before the result.

```text
kiva> eval 'print("hello", ARGV[1], ARGV[2])' world 42
hello	world	42
OK
```

### 23.4 Scripts are atomic

Everything a script writes is committed together when it ends. If it fails, nothing is kept:

```text
kiva> eval 'kiva.set("half", 1); error("boom")'
Error: ScriptError: runtime error: [string "script"]:1: boom
kiva> has half
No: Key 'half' does not exist.
```

If another session modified one of the same keys while the script was running, the script fails with `ERR_TRANSACTION_CONFLICT` and can be run again. A script run inside your own `begin ... commit` joins your transaction.

### 23.5 The sandbox

Scripts cannot harm the machine:

* No access to files, the operating system or the network: `io`, `os`, `require`, `load`, `dofile` do not exist.
* A time limit (`scripting.timeout_ms`, 5 seconds): `Error: ScriptError: runtime error: script exceeded the time limit of 5000 ms`.
* A memory limit (`scripting.memory_limit_mb`, 64 MB).

Available Lua libraries: `string`, `table`, `math`, `utf8`. Running scripts requires the right to write (role `editor` or `admin`).

### 23.6 Stored scripts

Write the script in a file, for example `increment.lua`:

```lua
local key = ARGV[1]
local by = tonumber(ARGV[2] or "1")
return kiva.incr(key, by)
```

Load it once under a name, then call it by that name from anywhere:

```text
kiva> script load "C:\scripts\increment.lua" as "inc"
Script 'inc' stored (82 bytes).
kiva> script run "inc" visits 5
6
kiva> script list
inc                      82 bytes
```

| Command | Effect |
| :--- | :--- |
| `script load "<file>" as "<name>"` | Stores (or replaces) a script. Admin only |
| `script run "<name>" [arguments]` | Runs it |
| `script list` | Lists stored scripts |
| `script show "<name>"` | Displays the source |
| `script delete "<name>"` | Removes it. Admin only |

The file is read by the KivaDB process, on the machine where it runs: give a full path. A script with a syntax error is refused at load time. Stored scripts are shared by all workspaces.

### 23.7 Triggers

A **trigger** runs a stored script automatically when keys change.

```text
create trigger [name] on set|update|del "<pattern>" run "<script>"
```

| Event | Fires when |
| :--- | :--- |
| `set` | A matching key is created |
| `update` | A matching existing key is overwritten |
| `del` | A matching key is deleted |

The script receives `ARGV[1]` = the event and `ARGV[2]` = the key. Example: keep a journal of user creations. File `audit_log.lua`:

```lua
kiva.set("log:" .. ARGV[1] .. ":" .. ARGV[2], "seen")
```

```text
kiva> script load "C:\scripts\audit_log.lua" as "audit_log"
Script 'audit_log' stored (54 bytes).
kiva> create trigger watch_users on set "user:*" run "audit_log"
Trigger 'watch_users' created: on set "user:*" run "audit_log".
kiva> set user:9 "new"
OK: user:9 saved.
kiva> has log:set:user:9
Yes: Key 'log:set:user:9' exists.
kiva> triggers
watch_users      on set    "user:*" run "audit_log"
kiva> drop trigger watch_users
Trigger 'watch_users' dropped.
```

Triggers run just **after** the change, in the background: the command that caused them does not wait, and a failing trigger does not undo it (the failure is recorded in the audit log). Writes made by a trigger do not fire triggers, so they cannot loop.

### 23.8 Measuring and debugging

```text
kiva> bench script "inc" iterations 1000 counter
SCRIPT inc: 1000 ops in 0.284s -> 3521 ops/sec (avg 0.284 ms)
```

Starting KivaDB with `--lua-debug` makes every script show the lines it executes (`[lua-debug] line 3`), which helps to follow its path.

---

## Chapter 24 — Monitoring

### 24.1 Quick looks

* `stats` in a shell ([16.1](#161-stats--the-dashboard)).
* `GET /v1/stats` for the same figures as JSON.
* `GET /health` to check that the server answers.
* `kivadb status` to check that the process is running.

### 24.2 Prometheus

`GET /metrics` on the REST port returns metrics in the Prometheus text format:

```text
# HELP kiva_requests_total Operations processed, by type.
# TYPE kiva_requests_total counter
kiva_requests_total{type="get"} 2
kiva_requests_total{type="set"} 1
kiva_requests_total{type="del"} 0
kiva_requests_total{type="other"} 0
# HELP kiva_keys Live keys in the workspace.
# TYPE kiva_keys gauge
kiva_keys 2
...
```

| Metric | Meaning |
| :--- | :--- |
| `kiva_requests_total{type=...}` | Operations processed: `get`, `set`, `del`, `other` |
| `kiva_request_duration_seconds_sum` / `_count` | Total time spent executing commands, and how many |
| `kiva_errors_total` | Commands that ended in an error |
| `kiva_http_requests_total` | HTTP requests received |
| `kiva_failed_logins_total` | Rejected logins |
| `kiva_keys` | Keys in the workspace |
| `kiva_storage_bytes` | Size of the data file |
| `kiva_storage_live_bytes` | Bytes of current data |
| `kiva_storage_fragmentation_ratio` | Dead share of the file, between 0 and 1 |
| `kiva_memory_bytes` | Estimated index memory |
| `kiva_checksum_errors_total` | Damaged records detected |
| `kiva_compactions_total` | Compactions done |
| `kiva_connected_clients` | Open TCP connections |
| `kiva_replicas_connected` | Replicas attached |
| `kiva_replication_lag_ops` | Operations the slowest replica is behind |
| `kiva_uptime_seconds` | Time since start |

A Prometheus configuration to collect them:

```yaml
scrape_configs:
  - job_name: 'kivadb'
    metrics_path: /metrics
    static_configs:
      - targets: ['localhost:8080']
```

Storage metrics describe the workspace of the user making the request (the `default` one for kiva). With authentication on, give Prometheus an account (`basic_auth`).

### 24.3 What to watch

| Signal | What it suggests |
| :--- | :--- |
| `kiva_checksum_errors_total` rises | The disk or the file is being damaged: run `check`, look at the hardware |
| `kiva_storage_fragmentation_ratio` stays high | Auto-compaction is off or cannot keep up |
| `kiva_replication_lag_ops` keeps growing | A replica is too slow or the link is poor |
| `kiva_failed_logins_total` rises fast | Someone is guessing passwords |
| `kiva_memory_bytes` near your RAM | Too many keys for this machine |

---

## Chapter 25 — Using KivaDB inside a Rust program

Besides connecting to a running KivaDB ([21.5](#215-rust)), a Rust program can contain the engine itself: no server, no network, the data files are opened directly by your program. This is the **embedded** mode.

### 25.1 The storage engine alone

```rust
use kivadb::{Engine, EngineOptions, Value};

fn main() -> Result<(), kivadb::KivaError> {
    let db = Engine::open("data/store.kiva", EngineOptions::default())?;

    db.put("greeting", &Value::Str("hello".into()), 0, false)?;
    if let Some((value, entry)) = db.get("greeting")? {
        println!("{} ({})", value.display(), entry.ty.name());
    }

    for (key, _) in db.range(Some("a"), Some("m"), false, 10) {
        println!("{key}");
    }

    db.delete("greeting")?;
    db.close();
    Ok(())
}
```

* `put(key, &value, expires_at, msgpack)`: `expires_at` is a Unix time in seconds (0 = never); `msgpack` chooses the encoding of documents.
* `Engine` can be shared between threads (`open` returns an `Arc`).
* Also available: `apply_batch` (atomic groups), `compact`, `check`, `create_index`, `query`, `stats`.

`EngineOptions` controls compression, auto-compaction and durability:

```rust
use kivadb::compress::Compression;

let options = EngineOptions {
    compression: Compression::Lz4,
    sync_writes: true,
    ..Default::default()
};
```

### 25.2 The full command language

`Instance` is a whole KivaDB (home directory, configuration, users, workspaces); `Session` is one user working in it.

```rust
use kivadb::{Instance, Options, Session};
use std::path::Path;

fn main() -> Result<(), kivadb::KivaError> {
    let instance = Instance::open(Path::new("./my-home"), Options::default())?;
    let mut session = Session::new(instance.clone(), true, None);

    let out = session.execute(r#"set user:1 {"name": "Alice"}"#);
    println!("{}", out.text());                 // OK: user:1 saved.
    println!("{}", session.execute("get user:1.name").text());

    instance.shutdown();
    Ok(())
}
```

`execute` returns the lines the command printed (`out.lines`, each flagged as error or not) and `out.has_err()` tells whether it failed.

Only one process can open a given data file or home directory at a time. If KivaDB is already running on that home, connect to it with the client instead.

---

## Chapter 26 — Troubleshooting

### 26.1 Starting

| What you see | Cause and remedy |
| :--- | :--- |
| `KivaDB is already running (pid ...)` | Nothing to do: it is started. `kivadb` opens a shell on it |
| `Error: cannot listen on 127.0.0.1:2911: ...` | Another program uses the port. Stop it, or choose another: `kivadb start --port 2912` (or `config set network.port 2912`) |
| `[WARN] REST API not started: cannot listen on ...:8080` | Port 8080 is taken. KivaDB works; only the REST API is missing. Use `--http-port` or `config set http.port` |
| `Error: refusing to listen on ... without authentication.` | You set a non-local `bind` without securing KivaDB. See [15.7](#157-opening-kivadb-to-the-network) |
| `Error: database is locked by another process` | Another process has these files open. Check `kivadb status` |
| `Error: invalid database file: missing KIVA signature` | The file is not a KivaDB data file. KivaDB does not touch it |
| `Error: invalid database file: unsupported storage format N` | The file was written by an incompatible version of KivaDB |
| `kivadb start` says the server stopped right away | The reason is in the last lines of `server.log`, which are displayed |

### 26.2 Commands

| Message | Meaning |
| :--- | :--- |
| `Key 'x' already exists. Use 'update' to change it.` | `set` only creates |
| `Key 'x' not found. Use 'set' to create it.` | `update` only modifies |
| `String values like 'x' must be quoted` | Put text between quotes |
| `Numbers and Booleans must not be quoted.` | You gave a type prefix `number` or `boolean` with a quoted value |
| `Cannot update [number] with a value formatted as [string].` | `update` keeps the type; use `change` to migrate |
| `InvalidKeyName: '44' cannot be purely numeric.` | Add a letter: `id44` |
| `'get' is a reserved keyword and cannot be used as a key.` | Use backticks: `` `get` `` |
| `Syntax Error: Unclosed quote detected (").` | A quote is missing |
| `Invalid JSON literal: ...` | The array or object is not valid JSON (field names need double quotes) |
| `Unknown command: 'x'. Try 'help'.` | Typing mistake |
| `ERR_TRANSACTION_CONFLICT: ...` | Someone changed the same key; redo the transaction |
| `ERR_DATA_CORRUPTED: ...` | The record is damaged on disk; see [16.4](#164-integrity-check) |
| `READONLY: this node is a replica` | Write on the master |

### 26.3 Access

| Message | Meaning |
| :--- | :--- |
| `permission denied: authentication required. Use: auth <user> <password>` | Log in first |
| `permission denied: invalid username or password` | Wrong credentials |
| `permission denied: this command requires the admin role` | Ask an administrator |
| `permission denied: the viewer role is read-only` | Your role cannot write |
| `permission denied: you only have read access to workspace 'x'` | You were granted *read* only |
| `permission denied: no access to workspace 'x'` | An admin must `grant` you access |
| `permission denied: accounts with an empty password can only log in from the local shell` | Give the account a password |

### 26.4 Other situations

**"I changed a key in one terminal and the other does not see it."** Both terminals must use the same home directory. `kivadb status` and `workspace current` show where each one is. Also check that you are not inside an uncommitted transaction.

**"My data is not where I expect."** Data lives in the home directory ([1.4](#14-where-kivadb-keeps-its-files)), not in the folder you launched `kivadb` from. `stats` shows the exact path on its last line.

**"The data file keeps growing."** It is fragmentation; see `stats`. Run `compact`, and check `autocompact status`.

**"KivaDB uses a lot of memory."** All keys are kept in RAM. Shorter keys, fewer keys, or documents grouping several small values reduce it.

**"A key disappeared."** It may have had a TTL. `scan` shows remaining times for keys that expire.

**"I forgot the kiva password."** See [15.10](#1510-emergency-access).

---

## Chapter 27 — Command cheat sheet

### Command line

| Command | Effect |
| :--- | :--- |
| `kivadb start` | Start the database in the background |
| `kivadb stop` | Stop it |
| `kivadb status` | Is it running? |
| `kivadb` | Open a shell |
| `kivadb serve` | Run the server in the foreground |
| `kivadb -e "<command>"` | Run a command and exit (repeatable) |
| `kivadb -w <workspace>` | Open a workspace |
| `kivadb -u <user> [-p <password>]` | Log in |
| `kivadb --connect <host:port>` | Shell on a remote KivaDB |
| `kivadb --home <folder>` | Use another home directory |
| `kivadb --skip-auth` | Start without authentication (maintenance) |
| `kivadb --lua-debug` | Trace Lua scripts |
| `kivadb start --resp-port <n>` / `--no-resp` | Redis-compatible port, or none |
| `kivadb --version`, `--help` | Version, options |

### Data

| Command | Effect |
| :--- | :--- |
| `set [type] <key> <value> [ttl <sec>]` | Create |
| `get [type] <key>` | Read (`get user:1.name` for a field) |
| `update [type] <key> <value>` | Replace, same type |
| `del [type] <key>` | Delete |
| `del all keys` | Empty the workspace (admin) |
| `has [type] <key>` | Does it exist? |
| `typeof <key>` | Its type |
| `change [type] <old> to [type] <new> [value]` | Rename or migrate |
| `bump [type] <key> add\|set <sec>` | Extend or reset the TTL |
| `print <expr>, <expr>` | Calculate and display |

### Listing and querying

| Command | Effect |
| :--- | :--- |
| `keys [like <pat>] [from <A> to <B>] [limit N] [reverse]` | Key names |
| `scan [pattern] [limit N]` | Keys with details |
| `range <A> to <B> [reverse] [limit N] [return f1, f2]` | Keys and values |
| `json get <key> <.path>` | Read a field |
| `json set <key> <.path> <value>` | Change a field |
| `json delete <key> <.path>` | Remove a field |
| `get where <pat>.<field> <op> <value> [limit N] [return f1, f2]` | Find documents |
| `count where ...` / `sum where ...` | Aggregates |
| `create index on <pat> (<field>)` | Create an index (admin) |
| `drop index on <pat> (<field>)` | Remove it (admin) |
| `indexes` | List indexes |
| `explain <query>` | Show the execution plan |
| `get similar <pat>.<field> to <vector> [limit N] [metric m] [return f1, f2]` | Vector similarity search |
| `create vector index on <pat> (<field>)` | Keep vectors in memory (admin) |
| `drop vector index on <pat> (<field>)` | Remove it (admin) |
| `convert <key> to msgpack\|json` | Change a document's encoding |

### Transactions

| Command | Effect |
| :--- | :--- |
| `begin` | Start |
| `commit` | Apply everything |
| `rollback` | Cancel everything |
| `savepoint <name>` | Place a bookmark |
| `rollback to <name>` | Return to a bookmark |
| `transaction status` | Show the open transaction |

### Maintenance

| Command | Effect |
| :--- | :--- |
| `stats` | Dashboard |
| `compact` | Reclaim disk space (admin) |
| `autocompact on\|off\|status\|threshold <N>` | Automatic compaction |
| `check` / `check repair` | Verify / repair |
| `hint info` | State of the index snapshot |
| `compress lz4\|zlib\|none` | Re-encode the workspace (admin) |
| `bench write <N>` / `bench read <N>` | Measure speed (admin) |
| `shutdown` | Stop the database (admin) |

### Workspaces and configuration

| Command | Effect |
| :--- | :--- |
| `workspace list` / `workspace current` | Show workspaces |
| `workspace switch <name>` | Change workspace |
| `workspace create <name> [path]` | Create (admin) |
| `workspace remove <name>` | Unregister (admin) |
| `config list` | All settings (admin) |
| `config get <key>` / `config set <key> <value>` | Read / change a setting (admin) |

### Users

| Command | Effect |
| :--- | :--- |
| `whoami` | Who am I, where |
| `auth <user> <password>` | Log in |
| `users` | List accounts (admin) |
| `create user "<n>" identified by "<p>" [role <r>]` | Create an account (admin) |
| `modify user "<n>" password "<p>"` | Change a password |
| `modify user "<n>" role "<r>"` | Change a role (admin) |
| `drop user "<n>"` | Delete an account (admin) |
| `grant read\|write\|admin on all\|workspace "<w>" to "<n>"` | Give access (admin) |
| `revoke read\|write\|admin [on all\|workspace "<w>"] from "<n>"` | Remove access (admin) |
| `audit last <N>` / `audit since "<date>"` / `audit failed-logins` | Read the audit log (admin) |

### Scripting and events

| Command | Effect |
| :--- | :--- |
| `eval '<lua>' [args]` | Run Lua code |
| `script load "<file>" as "<n>"` | Store a script (admin) |
| `script run "<n>" [args]` | Run a stored script |
| `script list` / `script show "<n>"` / `script delete "<n>"` | Manage scripts |
| `create trigger [name] on set\|update\|del "<pat>" run "<script>"` | Create a trigger (admin) |
| `drop trigger <name>` / `triggers` | Remove / list triggers |
| `bench script "<n>" iterations <N> [args]` | Time a script (admin) |
| `subscribe <pattern>` / `unsubscribe [pattern]` | Change notifications |

### Replication (admin)

| Command | Effect |
| :--- | :--- |
| `replica status` | State of replication |
| `replica of <host> <port>` / `replica of none` | Follow a master / become a master |
| `replica start` / `stop` / `sync` / `promote` | Control the link |
| `wait <N> [timeout_ms]` | Wait for N replicas |
| `failover to <replica>` | Hand over the master role |

### Shell

| Command | Effect |
| :--- | :--- |
| `help`, `h` | List of commands |
| `clear` | Clear the screen |
| `ping` | Answers `PONG` |
| `version` | Version of KivaDB |
| `exit`, `quit` | Leave the shell |

---

## Chapter 28 — Limits and scope

KivaDB 1.0.0 is a single-node database with replicas. Knowing what it does not do is as useful as knowing what it does.

**Not included in 1.0.0**

* **Encrypted connections.** No built-in TLS for port 2911 or for the REST API. Use a reverse proxy, a VPN or an SSH tunnel.
* **Automatic failover.** When a master dies, a person (or your own tooling) promotes a replica.
* **Clustering.** No sharding of data across machines, no quorum reads and writes, no Raft consensus, no distributed queries. Every node holds the whole database.
* **SQL and the PostgreSQL protocol.** The query language is the one described in this manual. Tools that require PostgreSQL (Prisma, DBeaver's SQL editor) cannot connect.
* **Approximate vector indexes (HNSW)** for many millions of vectors ([30.5](#305-how-fast-and-the-limits)).
* **The full Redis command set**: sets, sorted sets, streams and publish/subscribe are missing ([31.3](#313-supported-commands)).
* **A system service installer.** See [section 2.6](#26-starting-kivadb-with-the-computer).

**Design limits**

* All keys must fit in memory ([17.6](#176-limits)).
* Replication covers the `default` workspace only.
* A transaction lives in one connection; it cannot span several HTTP requests.
* Listing and query commands do not show the uncommitted writes of your own transaction.
* Strings have no escape sequences: a text cannot contain both kinds of quotes at once from the shell (it can through the REST API or the binary protocol).
* Changes to grants reach a connected user at their next login or workspace switch.

---

## Chapter 29 — Deploying KivaDB on a server

This chapter is for the day KivaDB leaves your own computer: a Linux machine at a hosting provider, a Windows server, a Mac, or a Docker container. The database is the same everywhere; what changes is how the system starts it and keeps it running.

### 29.1 What every deployment has in common

Whatever the system, a hosted KivaDB needs five things:

1. **The executable** `kivadb` (built for that system, see [Chapter 1](#chapter-1--installation)).
2. **A home directory** that survives restarts: configuration, users and data live there.
3. **A password** for the `kiva` administrator, and authentication turned on.
4. **A listening address** other machines can reach (`0.0.0.0` = every network interface).
5. **Open ports** in the firewall: 2911 for shells, Kiva Studio and client libraries; 8080 for the REST API if you use it.

KivaDB refuses to listen on a non-local address until point 3 is done ([15.7](#157-opening-kivadb-to-the-network)), so an open, password-less database cannot be published by accident.

### 29.2 Configuring with environment variables

On a server, settings are usually given through the environment rather than typed in a shell. `kivadb serve` and `kivadb start` read these variables:

| Variable | Effect | Same as |
| :--- | :--- | :--- |
| `KIVADB_HOME` | Home directory | `--home` |
| `KIVADB_BIND` | Address to listen on | `--bind`, `network.bind` |
| `KIVADB_PORT` | KivaDB protocol port | `--port`, `network.port` |
| `KIVADB_HTTP_PORT` | REST API port | `--http-port`, `http.port` |
| `KIVADB_ADMIN_PASSWORD` | Secures a new instance, see below | |

A command-line option wins over a variable, and a variable wins over `kiva.conf`.

**`KIVADB_ADMIN_PASSWORD`** does the two security steps for you when the server starts:

* if the `kiva` account still has an empty password, it receives this one;
* authentication is turned on.

It never overwrites a password that already exists: once you have changed the password with `modify user "kiva" password "..."`, the variable is ignored for the account. You can leave it in place or remove it.

```bash
KIVADB_ADMIN_PASSWORD='a-long-secret' KIVADB_BIND=0.0.0.0 kivadb serve
```

```text
  Security     : administrator password set, authentication turned on
KivaDB 1.0.0 server
  Home         : /var/lib/kivadb
  TCP protocol : 0.0.0.0:2911 (binary and plain text)
  REST API     : http://0.0.0.0:8080  (docs: /docs, metrics: /metrics, events: /ws)
  Auth         : required
  Replication  : master
Press Ctrl+C to stop.
```

### 29.3 Linux (systemd)

This is the usual case at a hosting provider. The steps below create a dedicated system user, install the executable and let systemd start KivaDB at boot and restart it if it stops.

```bash
sudo useradd --system --home-dir /var/lib/kivadb --create-home kivadb
sudo install -m 755 target/release/kivadb /usr/local/bin/kivadb
```

Put the password in a file only root can read, `/etc/kivadb.env`:

```ini
KIVADB_HOME=/var/lib/kivadb
KIVADB_BIND=0.0.0.0
KIVADB_ADMIN_PASSWORD=a-long-secret
```

```bash
sudo chmod 600 /etc/kivadb.env
```

Create `/etc/systemd/system/kivadb.service`:

```ini
[Unit]
Description=KivaDB database
After=network-online.target
Wants=network-online.target

[Service]
User=kivadb
Group=kivadb
EnvironmentFile=/etc/kivadb.env
ExecStart=/usr/local/bin/kivadb serve
Restart=on-failure
RestartSec=2
# Give KivaDB time to flush and write its index snapshot.
TimeoutStopSec=30
KillSignal=SIGINT
NoNewPrivileges=true
ProtectSystem=full

[Install]
WantedBy=multi-user.target
```

Start it and check:

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now kivadb
sudo systemctl status kivadb
sudo journalctl -u kivadb -f
```

Open the ports (example with `ufw`; skip 8080 if you do not use the REST API):

```bash
sudo ufw allow 2911/tcp
sudo ufw allow 8080/tcp
```

To run commands on the server itself, use the same home directory as the service:

```bash
sudo -u kivadb KIVADB_HOME=/var/lib/kivadb kivadb -u kiva
```

`systemctl stop kivadb` sends an interrupt; KivaDB shuts down cleanly. Use `serve` (not `start`) under systemd: the service manager wants the process to stay in the foreground.

### 29.4 Windows

Copy `kivadb.exe` to a fixed place, for example `C:\KivaDB\kivadb.exe`, and choose a data folder, for example `C:\KivaDB\data`.

Set the variables for the whole machine (an administrator terminal), then open a new terminal:

```bash
setx /M KIVADB_HOME "C:\KivaDB\data"
setx /M KIVADB_BIND "0.0.0.0"
setx /M KIVADB_ADMIN_PASSWORD "a-long-secret"
```

Have Windows start KivaDB when the machine boots, before anyone logs in:

```bash
schtasks /Create /TN "KivaDB" /TR "\"C:\KivaDB\kivadb.exe\" serve" /SC ONSTART /RU SYSTEM /RL HIGHEST
schtasks /Run /TN "KivaDB"
```

Allow the ports through the firewall:

```bash
netsh advfirewall firewall add rule name="KivaDB" dir=in action=allow protocol=TCP localport=2911
netsh advfirewall firewall add rule name="KivaDB REST" dir=in action=allow protocol=TCP localport=8080
```

Check with `kivadb status`, stop with `kivadb stop -u kiva` (it asks for the password). Once the first start has set the password, you may delete the `KIVADB_ADMIN_PASSWORD` variable.

On your own PC, for development, none of this is needed: `kivadb start` is enough.

### 29.5 macOS

For development, `kivadb start` is enough. To have KivaDB start with the Mac, describe it to `launchd` in `~/Library/LaunchAgents/dev.foma.kivadb.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.foma.kivadb</string>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/local/bin/kivadb</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>/tmp/kivadb.log</string>
  <key>StandardErrorPath</key><string>/tmp/kivadb.log</string>
</dict>
</plist>
```

```bash
launchctl load ~/Library/LaunchAgents/dev.foma.kivadb.plist
```

Data stays in `~/.kivadb`. To accept connections from other machines, add an `EnvironmentVariables` dictionary with `KIVADB_BIND` and `KIVADB_ADMIN_PASSWORD`, as on Linux.

### 29.6 Docker

The repository contains a `Dockerfile` and a `docker-compose.yml`. The image holds only the `kivadb` executable on a small Debian base; it runs as an unprivileged user and keeps everything in `/data`.

**Build the image** (from the `kiva-db` folder):

```bash
docker build -t kivadb:1.0.0 .
```

**Run it:**

```bash
docker run -d --name kivadb -e KIVADB_ADMIN_PASSWORD=a-long-secret -p 2911:2911 -p 8080:8080 -v kivadb-data:/data kivadb:1.0.0
```

| Part | Meaning |
| :--- | :--- |
| `-d --name kivadb` | Run in the background, under the name `kivadb` |
| `-e KIVADB_ADMIN_PASSWORD=...` | Password of the `kiva` administrator, installed at the first start. **Required**: inside a container KivaDB listens on every interface, so it insists on authentication |
| `-p 2911:2911` | Publish the KivaDB protocol port |
| `-p 8080:8080` | Publish the REST API (leave out if unused) |
| `-v kivadb-data:/data` | Keep the data in a named volume, so it survives the container |

Without the password the container stops at once and its log says why:

```text
Error: refusing to listen on 0.0.0.0 without authentication.
```

**Use it.** From Kiva Studio or any client: the address of the machine, port 2911, user `kiva`, your password. A shell inside the container:

```bash
docker exec -it kivadb kivadb -u kiva
```

Everyday commands:

```bash
docker logs -f kivadb
docker stop kivadb
docker start kivadb
```

`docker stop` lets KivaDB shut down cleanly. The image declares a health check (`kivadb status`), visible in `docker ps`.

**With Compose.** The provided file does the same as the `docker run` line above:

```bash
KIVADB_ADMIN_PASSWORD=a-long-secret docker compose up -d
```

On Windows PowerShell, set the variable first: `$env:KIVADB_ADMIN_PASSWORD = "a-long-secret"`, then `docker compose up -d`.

**Backup.** The volume holds everything. Stop the container, then archive the volume:

```bash
docker stop kivadb
docker run --rm -v kivadb-data:/data -v "$PWD":/backup debian:bookworm-slim tar czf /backup/kivadb-backup.tar.gz -C /data .
docker start kivadb
```

**Upgrade.** Build the new image, remove the container, run it again with the same volume. The data is untouched:

```bash
docker rm -f kivadb
```

then the `docker run` line again.

### 29.7 Before exposing KivaDB to the Internet

| Check | Why |
| :--- | :--- |
| `kiva` has a long, unique password | It is the administrator of everything |
| One account per person or application, with the smallest role that works | A leaked `viewer` password cannot destroy data |
| Only the needed ports are open, ideally only to known addresses | Fewer doors |
| Traffic goes through a VPN, an SSH tunnel or a TLS proxy | KivaDB 1.0.0 does not encrypt connections ([15.7](#157-opening-kivadb-to-the-network)) |
| Backups exist and were restored once as a test | A backup that was never restored is a hope, not a backup |
| `audit failed-logins` and the metrics are looked at | To notice password guessing early |

An SSH tunnel is the simplest safe way to reach a remote KivaDB without opening its port at all. On your own computer:

```bash
ssh -L 2911:localhost:2911 you@your-server
```

While that command runs, connect Kiva Studio to `localhost`, port 2911: the traffic travels encrypted inside SSH, and the server can keep listening on `127.0.0.1` only.

---

## Chapter 30 — Vector search for AI

### 30.1 What a vector is, and why store one

AI models turn a text, an image or a sound into an **embedding**: a list of numbers, typically a few hundred long, such as `[0.12, -0.40, 0.88, ...]`. Two embeddings that are close describe things that mean something close. Finding "the documents closest to this one" is how chatbots find the passages that answer a question (RAG), how search engines find results by meaning rather than by exact words, and how recommendations work.

In KivaDB, an embedding is simply stored as an array inside a document:

```text
kiva> set doc:1 {"title": "Kinshasa travel guide", "embedding": [0.9, 0.1, 0.0]}
kiva> set doc:2 {"title": "Goma lake trips", "embedding": [0.8, 0.3, 0.1]}
kiva> set doc:3 {"title": "Rust programming", "embedding": [0.0, 0.2, 0.95]}
```

Real embeddings come from a model (OpenAI, Mistral, a local model...) and have 384, 768, 1536 or more numbers; three are used here to keep the examples readable. Every vector compared in one search must have the same length.

### 30.2 Searching

```text
get similar <pattern>.<field> to <vector> [limit N] [metric cosine|dot|l2] [return f1, f2]
```

```text
kiva> get similar doc:*.embedding to [1, 0, 0] limit 2 return title
doc:1: 0.9939 {"title":"Kinshasa travel guide"}
doc:2: 0.9300 {"title":"Goma lake trips"}
(2 results, cosine metric, 3 vectors compared, documents read from disk)
```

Each line gives the key, the score, and the document (or the fields named after `return`). Results come best first; `limit` defaults to 10.

**"More like this".** Instead of a list of numbers, give the vector of another document:

```text
kiva> get similar doc:*.embedding to doc:3.embedding limit 2 metric l2 return title
doc:3: 0.0000 {"title":"Rust programming"}
doc:2: 1.1715 {"title":"Goma lake trips"}
(2 results, l2 metric, 3 vectors compared, documents read from disk)
```

### 30.3 Metrics

| Metric | Measures | Best score | Use when |
| :--- | :--- | :--- | :--- |
| `cosine` (default) | The angle between the vectors | 1 (same direction); -1 is opposite | Almost always: text embeddings are meant for it |
| `dot` | The dot product | Higher is closer | The model's documentation asks for it (vectors already normalised) |
| `l2` | The straight-line distance | 0 (identical); lower is closer | Image or numeric features where distance matters |

### 30.4 Vector indexes

Without an index, a search reads every matching document from disk. A **vector index** keeps the vectors of a field in memory, as 32-bit floats, and keeps them up to date on every write:

```text
kiva> create vector index on doc: (embedding)
Vector index created on doc:* (embedding): 3 vectors in memory.
kiva> get similar doc:*.embedding to [0, 0, 1] limit 1
doc:3: 0.9785 {"title":"Rust programming","embedding":[0.0,0.2,0.95]}
(1 results, cosine metric, 3 vectors compared, vector index)
kiva> indexes
doc:* (embedding) - vector index, 3 vectors, 3 dimensions
```

* The first vector indexed fixes the number of dimensions. Documents whose vector has another length are skipped and counted (`indexes` shows how many).
* Memory needed: about 4 bytes per number, so 100 000 vectors of 768 dimensions take about 300 MB.
* The definition is saved; the index is rebuilt when KivaDB starts. `drop vector index on doc: (embedding)` removes it.

### 30.5 How fast, and the limits

The search is **exact**: every candidate vector is compared, so the results are always the true closest ones. On an ordinary computer, with an index, a search over 3 000 vectors of 384 dimensions takes a few milliseconds; over 100 000 vectors, expect tens of milliseconds. That covers the memory of an assistant, a documentation search or a product catalogue.

For millions of vectors an **approximate** index (HNSW) is the usual answer: it gives up a little accuracy for much more speed. KivaDB 1.0.0 does not have one yet; see the roadmap.

### 30.6 From programs

The structured request `similar` returns the same results as JSON (`{"op": "similar", "target": "doc:*.embedding", "vector": [...], "limit": 5, "metric": "cosine"}`), on port 2911 or with `POST /v1/api`. The Node.js driver has `db.similar(target, vector, {limit, metric})`, and Kiva Studio has a Similarity tab in its Query screen.

---

## Chapter 31 — Redis compatibility

### 31.1 What it gives you

Redis is the most used key-value server; thousands of applications and libraries speak its protocol (RESP). KivaDB speaks it too, on port **6379**, so an application written for Redis can use KivaDB **by changing only its connection address**. Two official Redis libraries, node-redis and ioredis, were run unchanged against KivaDB to check this.

```bash
redis-cli -p 6379
```

```text
127.0.0.1:6379> SET greeting "hello"
OK
127.0.0.1:6379> INCR visits
(integer) 1
127.0.0.1:6379> HSET user:1 name Alice city Kinshasa
(integer) 2
```

The data is the same as everywhere else in KivaDB:

```text
kiva> get user:1.name and visits
user:1.name: Alice [string]
visits: 1 [number]
```

### 31.2 How types match

| Redis | KivaDB |
| :--- | :--- |
| string | `string` when the bytes are UTF-8 text, `bytes` otherwise. `INCR` and friends store a `number` |
| hash | `object` whose fields hold strings |
| list | `array` of strings |

KivaDB values read through Redis: strings, numbers, booleans, null and bytes come back as Redis strings; objects are hashes and arrays are lists (`GET` on them answers `WRONGTYPE`, as Redis does).

### 31.3 Supported commands

| Group | Commands |
| :--- | :--- |
| Connection | `PING`, `ECHO`, `AUTH`, `HELLO` (protocol 2), `SELECT 0`, `CLIENT`, `QUIT`, `COMMAND`, `INFO` |
| Strings | `GET`, `SET` (with `EX`, `PX`, `EXAT`, `PXAT`, `NX`, `XX`, `KEEPTTL`, `GET`), `SETNX`, `SETEX`, `PSETEX`, `GETSET`, `GETDEL`, `MGET`, `MSET`, `APPEND`, `STRLEN`, `INCR`, `DECR`, `INCRBY`, `DECRBY`, `INCRBYFLOAT` |
| Keys | `DEL`, `UNLINK`, `EXISTS`, `TYPE`, `KEYS`, `SCAN`, `RENAME`, `EXPIRE`, `PEXPIRE`, `EXPIREAT`, `PEXPIREAT`, `TTL`, `PTTL`, `PERSIST`, `DBSIZE`, `FLUSHDB`, `FLUSHALL` |
| Hashes | `HSET`, `HMSET`, `HSETNX`, `HGET`, `HMGET`, `HGETALL`, `HKEYS`, `HVALS`, `HLEN`, `HEXISTS`, `HDEL`, `HINCRBY` |
| Lists | `LPUSH`, `RPUSH`, `LPOP`, `RPOP`, `LLEN`, `LRANGE`, `LINDEX` |
| Transactions | `MULTI`, `EXEC`, `DISCARD` |

Pipelining works. Not supported: sets, sorted sets, streams, publish/subscribe, Lua `EVAL` through Redis, `WATCH`, RESP3, cluster commands. An unsupported command answers `ERR unknown command`, so an application finds out immediately.

### 31.4 Logins, workspaces, roles

Everything KivaDB enforces applies here too:

* With authentication on, the client must log in. `AUTH <password>` logs in as `kiva` (Redis's "default" user); `AUTH <user> <password>` as any account. In a URL: `redis://kiva:password@host:6379`.
* A connection works in the user's workspace; `SELECT 0` is accepted, other numbers are refused (KivaDB workspaces have names).
* A viewer cannot write (`NOPERM`), a replica refuses writes (`READONLY`), `FLUSHDB` needs the admin role.
* `MULTI ... EXEC` runs as a KivaDB transaction: all or nothing, and `EXEC` answers nil if another session changed the same keys meanwhile.

### 31.5 Configuration

| Setting | Default | |
| :--- | :--- | :--- |
| `resp.enabled` | `true` | Start the Redis listener with `kivadb start` / `serve` |
| `resp.port` | `6379` | Its port; also `--resp-port`, `KIVADB_RESP_PORT` |

`--no-resp` turns it off for one start. If a real Redis already uses port 6379 on the machine, KivaDB says so in its log and carries on without the listener; choose another port.

---

## Chapter 32 — Keeping values in memory (value cache)

By default only keys live in memory; every read takes the value from the data file (usually from the operating system's file cache, which is fast but not free). For data read again and again - sessions, configuration, hot product pages - KivaDB can keep **recently read values** in memory too.

```text
kiva> config set cache_values true
OK: storage.cache_values = true
kiva> config set cache_size_mb 512
OK: storage.cache_size_mb = 512
```

* The budget is per workspace. When it is full, the values read least recently are dropped.
* The file stays the truth: the cache only holds copies, checked against the version of each record, so it can never return an old value. A write, a delete or a transaction replaces or removes the copy.
* It takes effect immediately and can be turned off at any time (`config set cache_values false`).
* `stats` shows how it is doing:

```text
 > Value Cache          : 160 bytes of 256.00 MB used, 1 values, 50.0% hits
```

A high hit rate means the cache is useful; a low one on a large database means the reads are spread over too many keys for the budget.

Durability is unchanged: writes go to disk exactly as before. This is a read cache, not a memory-only database.

---

## Glossary

| Term | Meaning |
| :--- | :--- |
| **Append-only log** | A file to which records are only ever added at the end |
| **Atomic** | Happens completely or not at all |
| **Checksum (CRC32)** | A number computed from data, stored with it, and recomputed at read time to detect damage |
| **Commit** | Apply the changes of a transaction |
| **Compaction** | Rewriting the data file without obsolete records |
| **Document** | A value with named fields (object) or a list (array) |
| **Fragmentation** | The share of the data file occupied by obsolete records |
| **Grant** | Permission given to a user on a workspace |
| **Hint file** | A saved copy of the index, for fast start |
| **Home directory** | The folder holding KivaDB's configuration, users and data |
| **Index** | The in-memory table telling where each key's value is on disk |
| **Key** | The name under which a value is stored |
| **Master / replica** | The node that receives writes / a node that copies it |
| **MessagePack** | A compact binary encoding of JSON-like data |
| **Path** | The address of a field inside a document: `.address.city` |
| **Pattern** | A key template with `*` and `?` |
| **Projection** | Keeping only some fields of the documents returned |
| **Role** | What a user is allowed to do: admin, editor, viewer |
| **Rollback** | Cancel the changes of a transaction |
| **Savepoint** | A bookmark inside a transaction |
| **Secondary index** | An index on a field of documents, to speed up queries |
| **Tombstone** | The record that marks a key as deleted |
| **Transaction** | A group of changes applied together |
| **Trigger** | A script run automatically when keys change |
| **TTL** | Time to live: the delay after which a key expires |
| **Workspace** | An independent database inside one KivaDB |
