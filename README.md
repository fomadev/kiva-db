# KivaDB (v2.1.0)

KivaDB is a lightweight, high-performance NoSQL Key-Value database engine built with a hybrid C/C++ architecture[cite: 9]. It combines the low-level efficiency of C for storage operations with the power of C++ STL for advanced indexing and TTL (Time To Live) management[cite: 3, 9].

Designed for speed and simplicity, KivaDB utilizes an Append-Only File (AOF) storage strategy and a memory-mapped index to ensure high read performance[cite: 8, 9].

## Key Features

* **Hybrid Engine**: Core storage and transaction management implemented in C11, with optimized indexing in C++17[cite: 9].
* **Persistent Storage**: Data persistence across restarts using a robust binary format with a signature-based header (`KIVA`).
* **Smart TTL Support**: Native support for expiring keys, managed via lazy deletion during boot and data access.
* **Type Safety**: Built-in type inference and validation for String, Number, and Boolean types.
* **Advanced Shell**: A feature-rich CLI supporting quoted strings (`""`, `''`), reserved keyword protection, and command chaining[cite: 4, 7].
* **Maintenance Suite**: Integrated tools for database scanning, real-time statistics, and automatic compaction to optimize disk usage[cite: 3, 4].

## Architecture

KivaDB is organized into three distinct functional layers:

1. **The Shell (CLI)**: Manages user input, command parsing, and request validation[cite: 9].
2. **The Index (C++)**: A `std::map` based index storing key metadata and file offsets for logarithmic-time lookups[cite: 3, 9].
3. **The Storage (C)**: Manages binary I/O operations, file format integrity (V2), and the append-only log.

## Getting Started

### Prerequisites
* `gcc` (C11 support)
* `g++` (C++17 support)
* `make`

### Installation
The project includes a unified Makefile for compilation:
```bash
git clone https://github.com/fomadev/KivaDB.git
cd KivaDB
make clean
make
```

### Running the Shell
Once compiled, launch the executable:
```bash
./kivadb
```

## Command Usage

<table>
    <thead>
        <tr>
            <th>Command</th>
            <th>Description</th>
            <th>Example</th>
        </tr>
    </thead>
    <tbody>
        <tr>
            <td><strong>set</strong></td>
            <td>Create a new key with optional TTL</td>
            <td><code>set u1 "Fordi" ttl 3600</code></td>
        </tr>
        <tr>
            <td><strong>get</strong></td>
            <td>Retrieve values for one or more keys</td>
            <td><code>get u1 and u2</code></td>
        </tr>
        <tr>
            <td><strong>update</strong></td>
            <td>Modify an existing key's value</td>
            <td><code>update u1 "New Value"</code></td>
        </tr>
        <tr>
            <td><strong>change</strong></td>
            <td>Rename an existing key to a new name</td>
            <td><code>change old_key to new_key</code></td>
        </tr>
        <tr>
            <td><strong>typeof</strong></td>
            <td>Identify the data type of a key</td>
            <td><code>typeof u1</code></td>
        </tr>
        <tr>
            <td><strong>del</strong></td>
            <td>Delete specific keys or all keys</td>
            <td><code>del u1</code> or <code>del all keys</code></td>
        </tr>
        <tr>
            <td><strong>scan</strong></td>
            <td>Display all active keys and metadata</td>
            <td><code>scan</code></td>
        </tr>
        <tr>
            <td><strong>stats</strong></td>
            <td>View database health and storage metrics</td>
            <td><code>stats</code></td>
        </tr>
        <tr>
            <td><strong>compact</strong></td>
            <td>Reorganize storage to reclaim space</td>
            <td><code>compact</code></td>
        </tr>
        <tr>
            <td><strong>clear</strong></td>
            <td>Clear the terminal screen</td>
            <td><code>clear</code></td>
        </tr>
    </tbody>
</table>

## File Format (v2.1)
KivaDB uses a structured binary format to ensure data reliability and rapid recovery[cite: 3, 8]:

### Header (12 bytes)
* **Signature** (4 bytes): `KIVA`

* **Version** (4 bytes): Format version (V1 or V2)  

* **Reserved** (4 bytes): Aligned for future extensions[cite: 3]

### Data Entry
* **k_size** (uint32): Length of the key  

* **v_size** (uint32): Length of the value (0 indicates a deletion marker)  

* **type** (uint8): Data type identifier (String, Number, Boolean)[cite: 3, 8]

* **expires_at** (int64): Unix timestamp for expiration (0 for permanent)[cite: 3, 8]

* **key**: Variable length key name  

* **value**: Variable length value data