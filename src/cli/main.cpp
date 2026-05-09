/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>

#include "parser.hpp"
#include "commands.hpp"

extern "C" {
    #include "../../include/kivadb.h"
    #include "../core/kivadb_internal.h"
}

// --- Gestion multiplateforme ---
#ifdef _WIN32
    #include <direct.h>
    #define CLEAR_COMMAND "cls"
    #ifndef MKDIR
        #define MKDIR(path) _mkdir(path)
    #endif
#else
    #include <sys/stat.h>
    #define CLEAR_COMMAND "clear"
    #ifndef MKDIR
        #define MKDIR(path) mkdir(path, 0777)
    #endif
#endif

/**
 * Affiche l'aide utilisateur pour les commandes du shell.
 */
void print_help() {
    std::cout << "\nAvailable commands:\n"
              << "  set <key> <value> [TTL]  : Set a key with optional TTL (seconds)\n"
              << "  get <key>                : Get the value of a key\n"
              << "  update <key> <new_val>   : Update an existing key\n"
              << "  del <key>                : Delete a key\n"
              << "  typeof <key>             : Show the type of a key\n"
              << "  change <old> to <new>    : Rename a key\n"
              << "  scan                     : List all keys in memory\n"
              << "  stats                    : Show database statistics\n"
              << "  compact                  : Rebuild data file to save space\n"
              << "  clear                    : Clear the terminal screen\n"
              << "  exit                     : Close the database and exit\n\n";
}

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    // Création du dossier de données
    MKDIR("data");
    const char* db_path = "data/store.kiva";

    // Ouverture de la base de données
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open or initialize database at " << db_path << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v2.1.1.5\nType 'help' or 'h' for command list\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        // Analyse de la ligne
        std::vector<char> delimiters;
        auto tokens = CommandParser::tokenize(line, delimiters);
        
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_dur = true;

        // --- ROUTAGE ET VALIDATION DES COMMANDES ---
        
        if (cmd == "set") {
            if (tokens.size() < 3) {
                std::cerr << "Error: Usage: set <key> <value> [ttl_sec]" << std::endl;
            } else {
                handle_set(&db, tokens, delimiters);
            }
        }
        else if (cmd == "update") {
            if (tokens.size() < 3) {
                std::cerr << "Error: Usage: update <key> <new_value>" << std::endl;
            } else {
                handle_update(&db, tokens, delimiters);
            }
        }
        else if (cmd == "get") {
            if (tokens.size() != 2) {
                std::cerr << "Error: Usage: get <key>" << std::endl;
            } else {
                handle_get(&db, tokens, delimiters);
            }
        }
        else if (cmd == "del") {
            if (tokens.size() != 2) {
                std::cerr << "Error: Usage: del <key>" << std::endl;
            } else {
                handle_del(&db, tokens, db_path);
            }
        }
        else if (cmd == "clear") {
            if (tokens.size() > 1) {
                std::cerr << "Error: 'clear' command does not accept arguments." << std::endl;
            } else {
                system(CLEAR_COMMAND);
            }
            show_dur = false;
        }
        else if (cmd == "change") {
            // Format attendu : change <old> to <new> (soit 4 tokens)
            if (tokens.size() != 4 || tokens[2] != "to") {
                std::cerr << "Error: Usage: change <old_key> to <new_key>" << std::endl;
            } else {
                handle_change(&db, tokens); 
            }
        }
        else if (cmd == "typeof") {
            if (tokens.size() != 2) {
                std::cerr << "Error: Usage: typeof <key>" << std::endl;
            } else {
                handle_typeof(&db, tokens);
            }
        }
        else if (cmd == "scan") {
            if (tokens.size() > 1) {
                std::cerr << "Error: 'scan' command does not accept arguments." << std::endl;
            } else {
                index_scan(db); 
            }
            show_dur = false;
        }
        else if (cmd == "stats") {
            if (tokens.size() > 1) {
                std::cerr << "Error: 'stats' command does not accept arguments." << std::endl;
            } else {
                handle_stats(&db); 
            }
            show_dur = false;
        }
        else if (cmd == "compact") {
            if (tokens.size() > 1) {
                std::cerr << "Error: 'compact' command does not accept arguments." << std::endl;
            } else {
                kiva_compact(db); 
                std::cout << "Database storage compacted successfully.\n";
            }
        }
        else if (cmd == "help" || cmd == "h") {
            if (tokens.size() > 1) {
                std::cerr << "Error: 'help' command does not accept arguments." << std::endl;
            } else {
                print_help(); 
            }
            show_dur = false;
        }
        else {
            std::cout << "Unknown command: '" << cmd << "'. Try 'help'.\n";
            show_dur = false;
        }

        // Affichage du temps d'exécution
        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    if (db) kiva_close(db);
    std::cout << "Goodbye.\n";
    
    return 0;
}