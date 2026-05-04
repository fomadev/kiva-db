#include <iostream>

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v2.0.1) ---\n"
              << "  set [type] `key` \"val\" [ttl s]   : Set ONLY if key doesn't exist\n"
              << "  update `key` \"val\"               : Update ONLY if key exists\n"
              << "  change `old` to `new`            : Rename key safely\n"
              << "  get `key1` and `key2`            : Retrieve values\n"
              << "  typeof `key`                     : Show data type\n"
              << "  del `key` OR del all keys        : Delete keys\n"
              << "  scan                             : List all entries\n"
              << "  compact | stats | exit           : Utility commands\n"
              << "-----------------------------------\n";
}