# KivaDB

KivaDB is an ultra-lightweight key-value store database engine written in C, inspired by the Bitcask architecture. It uses an append-only log and an in-memory hash table to ensure top performance.

## New features in v1.1.0 (Strict Update)
- **Strict & Manual Typing**: Possibility to force a type with `set string`, `set number` or `set boolean`.
- **Deterministic Parsing**: Strings must be enclosed in quotation marks `""`.
- **Secure Chaining**: The use of the keyword `and` is now mandatory for multiple operations, preventing accidental creations.
- **Improved Typeof**: Multi-key support for inspecting data types in a single line.
- **Automatic Detection**: Intelligent algorithm capable of distinguishing a number, a boolean or text without intervention.

## 🛠️ Installation & Compilation

Make sure you have `gcc` and `make` installed on your system.

```bash
git clone https://github.com/fomadev/kivadb.git
cd KivaDB
make clean
make
```

## Using the Shell

Launch the database :

```bash
./kivadb
```

## command available

<table>
<thead>
<tr>
<th>Command</th>
<th>Syntax / Strict Example</th>
<th>Description</th>
</tr>
</thead>
<tbody>
<tr>
<td><b>set</b></td>
<td><code>set string name "Foma" and number age 25</code></td>
<td>Creates new pairs. Supports explicit typing and required quotes for text.

<tr>
<td><b>get</b></td>
<td><code>get name and age</code></td>
<td>Retrieves the values ​​of one or more keys.</td>

<tr>
<td><b>update</b></td>
<td><code>update name "Fordi"</code></td>
<td>Modifies an existing key. Follows the same typing rules as SET.

<tr>
<td><b>typeof</b></td>
<td><code>typeof name and age</code></td>
<td>Displays the type (string, number, boolean) for one or more keys.</td>

<tr>
<td><b>del</b></td>
<td><code>del key1 and key2</code></td>
<td>Deletes one or more keys from the index and marks the entry as deleted from disk.</td>

<tr>
<td><b>scan</b></td>
<td><code>scan</code></td>
<td>Lists all keys present in RAM with their type and size binary.</td>
</tr>
<tr>
<td><b>compact</b></td>
<td><code>compact</code></td>
<td>Rewrites the <code>.kiva</code> file to remove old versions of keys and deleted data.</td>
</tr>
<tr>
<td><b>stats</b></td>
<td><code>stats</code></td>
<td>Displays the number of keys and the actual file size on disk.</td>

</tbody>
</table>

## Performance (Stress Test)

Tests performed on v1.1.0 confirm the robustness of the engine despite the addition of the following types of controls:

* **Volume**: 100,000 entries.

* **Total time**: ~3.82 seconds.

* **Average speed**: ~26,000 operations/second.

* **Stability**: Strict type management ensures the integrity of stored data.