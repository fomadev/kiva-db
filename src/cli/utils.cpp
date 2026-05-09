/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include <iostream>

/**
 * Affiche l'aide du Shell KivaDB avec les précisions sur le typage strict.
 * Mise à jour pour la v2.1.3 (Refactoring & Security update).
 */
void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v2.1.3 Strict Mode) ---\n"
              << "  RELIABILITY RULES:\n"
              << "  - Strings MUST be quoted: \"value\" or 'value'.\n"
              << "  - Numbers MUST NOT be quoted: 42, 3.14, true.\n"
              << "  - Booleans MUST NOT be quoted: true or false.\n"
              << "  - Self-rename (change u to u) is ignored to prevent data loss.\n"
              << "  - Type mismatch during rename is blocked without a new value.\n"
              << "\n  COMMANDS:\n"
              << "  set [type] <key> <val> [ttl <sec>]  : Save a value.\n"
              << "  update [type] <key> <val>           : Update value (type-safe).\n"
              << "  get [type] <key1> and <key2>        : Retrieve values.\n"
              << "  has [type] <key>                    : Check existence.\n"
              << "  del [type] <key> / del all keys     : Remove data.\n"
              << "  typeof <key>                        : Show the stored data type.\n"
              << "  change [t] <old> to [t] <new> [val] : Advanced Refactoring (Rename/Migrate).\n"
              << "  scan                                : List all keys with types and sizes.\n"
              << "  stats                               : Show DB file and memory statistics.\n"
              << "  compact                             : Reorganize storage and remove stale data.\n"
              << "  clear                               : Clear the terminal screen.\n"
              << "  exit                                : Safely close KivaDB and exit.\n"
              << "----------------------------------------------\n\n";
}