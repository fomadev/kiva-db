/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include <iostream>

/**
 * Affiche l'aide du Shell KivaDB avec les précisions sur le typage strict.
 * Mise à jour pour la v2.1.5 (Transactionnal Rename, Arithmetic & Interpolation).
 */
void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v2.1.7 Stable) ---\n"
              << "  RELIABILITY & SYNTAX RULES:\n"
              << "  - Strings        : MUST be quoted -> \"Hello\" or 'Kiva'.\n"
              << "  - Numbers/Bools  : MUST NOT be quoted -> 42, 3.14, true, false.\n"
              << "  - Key Names      : Cannot be purely numeric (e.g., '44' is invalid, 'id44' is OK).\n"
              << "  - Multi-Keys     : Use 'and' to chain operations -> get u and age.\n"
              << "  - Transactions   : Renaming ('change') is now atomic (no data loss on failure).\n"

              << "\n  ADVANCED FEATURES:\n"
              << "  - Arithmetic     : print (10 + 5) * 2 / (key_val - 1)\n"
              << "  - Interpolation  : print \"User age is ${age_user}\"\n"
              << "  - Separators     : Use commas in print -> print \"Name:\", user_1\n"

              << "\n  COMMANDS:\n"
              << "  set [t] <key> <val> [ttl <sec>]  : Save a value with optional expiration.\n"
              << "  update [t] <key> <val>           : Update existing value (type-safe).\n"
              << "  get [t] <k1> [and <k2>...]       : Retrieve one or multiple values.\n"
              << "  has [t] <k1> [and <k2>...]       : Check if keys exist in the database.\n"
              << "  del <key> | del all keys         : Remove specific data or wipe the DB.\n"
              << "  typeof <key>                     : Show the stored data type (string, number, bool).\n"
              << "  change [t] <old> to [t] <new> [v]: Advanced Rename/Migration (Transactional).\n"
              << "  print <expr1>, <expr2>           : Evaluate and display expressions/variables.\n"
              << "  scan                             : List all keys with their types and memory size.\n"
              << "  stats                            : Show database file health and memory usage.\n"
              << "  compact                          : Optimize storage and cleanup deleted entries.\n"
              << "  clear                            : Clear the terminal screen.\n"
              << "  exit                             : Safely flush buffers and close KivaDB.\n"
              << "--------------------------------------------------------------------------\n\n";
}