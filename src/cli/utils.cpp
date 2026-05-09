/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include <iostream>

/**
 * Affiche l'aide du Shell KivaDB avec les précisions sur le typage strict.
 */
void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v2.1.2 STL) ---\n"
              << "  RELIABILITY RULES:\n"
              << "  - Strings MUST be quoted: \"value\" or 'value'.\n"
              << "  - Numbers MUST NOT be quoted: 42, 3.14, true.\n"
              << "  - Booleans MUST NOT be quoted: true or false.\n"
              << "  - Reserved keywords (set, get, string, etc.) cannot be used as keys.\n"
              << "\n  COMMANDS:\n"
              << "  set [type] <key> <val> [ttl <sec>]  : Save a value. Quotes define strings.\n"
              << "  update [type] <key> <val>           : Update value (must match existing type).\n"
              << "  get [type] <key1> and <key2>        : Retrieve values (optional type check).\n"
              << "  has [type] <key>                    : Check if a key exists (optional type check).\n"
              << "  del [type] <key> / del all keys     : Remove data (optional type check).\n"
              << "  typeof <key>                        : Show the stored data type.\n"
              << "  change <old> to <new>               : Rename a key (new key must be unique).\n"
              << "  scan                                : List all keys with types and sizes.\n"
              << "  stats                               : Show DB file and memory statistics.\n"
              << "  compact                             : Reorganize storage and remove stale data.\n"
              << "  clear                               : Clear the terminal screen.\n"
              << "  exit                                : Safely close KivaDB and exit.\n"
              << "----------------------------------------\n\n";
}