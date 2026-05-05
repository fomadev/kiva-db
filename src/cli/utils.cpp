#include <iostream>

void print_help() {
    std::cout << "\n--- KivaDB Shell Help ---\n"
              << "  set [type] <key> <val> [ttl <sec>]  : Save a value (strictly typed).\n"
              << "  update [type] <key> <val>           : Update existing key (checks type compatibility).\n"
              << "  get <key1> and <key2>               : Retrieve values.\n"
              << "  del <key> / del all keys            : Remove data.\n"
              << "  typeof <key>                        : Show data type of a key.\n"
              << "  change <old> to <new>               : Rename a key.\n"
              << "  scan                                : List all indexed keys.\n"
              << "  stats                               : Database file statistics.\n"
              << "  compact                             : Reorganize storage to save space.\n"
              << "  clear                               : Clear shell screen.\n"
              << "  exit                                : Close KivaDB.\n"
              << "-------------------------\n\n";
}