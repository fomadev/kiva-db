# KivaDB (v2.0.0)

**KivaDB** is a lightweight, high-performance NoSQL Key-Value database engine built with a hybrid **C/C++ architecture**. It combines the low-level efficiency of C for storage with the power of C++ STL for advanced indexing and TTL (Time To Live) management.

Designed for speed and simplicity, KivaDB uses an append-only file storage (AOF) strategy and a Bitcask-inspired memory-mapped index to ensure read performance.

## Key Features

* **Hybrid Engine**: Core storage and transactions in C11, optimized indexing in C++17.

* **Persistent Storage**: Data survives restarts using a robust binary format with a signature-based header (`KIVA`).

* **Smart TTL Support**: Native support for expiring keys, handled via lazy deletion during boot and access

* **Type Safety**: Built-in type inference (String, Number, Boolean).

* Strict Shell: A feature-rich CLI with quote support (`""`, `''`, ``) and command chaining.

* **Self-Healing**: Automatic database compaction to reclaim disk space and upgrade file formats.

## Architecture

KivaDB is split into three main layers:

1. **The Shell (CLI)**: Handles user input and command parsing.

2. **The Index (C++)**: A ``std::map`` based index that stores key metadata and file offsets for instant lookups.

3. **The Storage (C)**: Manages binary I/O, file locking, and the append-only log.

## Getting Started

### Prerequisites
* `gcc` (C11 support)

* `g++` (C++17 support)

* `make`

### Installation

```bash
git clone https://github.com/fomadev/KivaDB.git
cd KivaDB
make
```

### Running the Shell

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
            <td><code>set</code></td>
            <td>Create a new key (fails if exists)</td>
            <td><code>set u1 "Fordi" ttl 3600</code></td>
        </tr>
        <tr>
            <td><code>update</code></td>
            <td>Modify an existing key</td>
            <td><code>update u1 "New Value"</code></td>
        </tr>
        <tr>
            <td><code>get</code></td>
            <td>Retrieve one or more values</td>
            <td><code>get u1 and u2</code></td>
        </tr>
        <tr>
            <td><code>change</code></td>
            <td>Rename a key safely</td>
            <td><code>change old_key to new_key</code></td>
        </tr>
        <tr>
            <td><code>del</code></td>
            <td>Delete keys or clear all</td>
            <td><code>del u1</code> or <code>del all keys</code></td>
        </tr>
        <tr>
            <td><code>scan</code></td>
            <td>List all active keys and metadata</td>
            <td><code>scan</code></td>
        </tr>
        <tr>
            <td><code>stats</code></td>
            <td>Show database health and size</td>
            <td><code>stats</code></td>
        </tr>
        <tr>
            <td><code>compact</code></td>
            <td>Optimize disk usage</td>
            <td><code>compact</code></td>
        </tr>
    </tbody>
</table>

## File Format (v2.0)

KivaDB uses a structured binary format for reliability:

* **Header**: 12-byte fixed header (`KIVA` signature + Version + Reserved).  

* **Data Entry**:

    * `k_size` (uint32)

    * `v_size` (uint32)

    * `type` (uint8)

    * `expires_at` (int64)

    * `key` (variable)

    * `value` (variable)