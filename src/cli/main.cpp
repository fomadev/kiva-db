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
void print_help();

/**
 * Vérifie si un jeton est un type KivaDB valide.
 */
bool is_kiva_type(const std::string& t) {
    return (t == "string" || t == "number" || t == "boolean");
}

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    // Initialisation de l'environnement
    MKDIR("data");
    const char* db_path = "data/store.kiva";

    // Ouverture de la base de données
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open or initialize database at " << db_path << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v2.1.1.5 (Strict Mode)\nType 'help' or 'h' for command list\n";

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
        size_t n = tokens.size();
        clock_t start = clock();
        bool show_dur = true;

        // --- ROUTAGE VERSION 2.1.1.5 (STRICT ERROR MODE) ---

        if (cmd == "set") {
            // Formats supportés: 
            // set k v (3)
            // set t k v (4) -> t type
            // set k v ttl s (5) -> tokens[3] == "ttl"
            // set t k v ttl s (6) -> t type ET tokens[4] == "ttl"
            bool ok = (n == 3) || 
                      (n == 4 && is_kiva_type(tokens[1])) || 
                      (n == 5 && tokens[3] == "ttl") || 
                      (n == 6 && is_kiva_type(tokens[1]) && tokens[4] == "ttl");
            
            if (ok) {
                handle_set(&db, tokens, delimiters);
            } else {
                std::cerr << "Error: Invalid set syntax.\nUsage: set [type] <key> <value> [ttl <sec>]" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "get") {
            // Formats: get k (2), get t k (3)
            bool ok = (n == 2) || (n == 3 && is_kiva_type(tokens[1]));
            if (ok) {
                handle_get(&db, tokens, delimiters);
            } else {
                std::cerr << "Error: Invalid get syntax.\nUsage: get [type] <key>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "update") {
            // Formats: update k v (3), update t k v (4)
            bool ok = (n == 3) || (n == 4 && is_kiva_type(tokens[1]));
            if (ok) {
                handle_update(&db, tokens, delimiters);
            } else {
                std::cerr << "Error: Invalid update syntax.\nUsage: update [type] <key> <value>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "del") {
            // Formats: del k (2), del t k (3)
            bool ok = (n == 2) || (n == 3 && is_kiva_type(tokens[1]));
            if (ok) {
                handle_del(&db, tokens, db_path);
            } else {
                std::cerr << "Error: Invalid del syntax.\nUsage: del [type] <key>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "typeof") {
            if (n == 2) {
                handle_typeof(&db, tokens);
            } else {
                std::cerr << "Error: Usage: typeof <key>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "change") {
            // Format strict: change <old> to <new>
            if (n == 4 && tokens[2] == "to") {
                handle_change(&db, tokens);
            } else {
                std::cerr << "Error: Usage: change <old_key> to <new_key>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "scan" || cmd == "stats" || cmd == "compact" || cmd == "clear") {
            if (n == 1) {
                if (cmd == "scan") index_scan(db);
                else if (cmd == "stats") handle_stats(&db);
                else if (cmd == "compact") { 
                    kiva_compact(db); 
                    std::cout << "Database storage compacted successfully." << std::endl; 
                }
                else if (cmd == "clear") system(CLEAR_COMMAND);
            } else {
                std::cerr << "Error: '" << cmd << "' command does not accept arguments." << std::endl;
            }
            show_dur = false;
        }
        else if (cmd == "help" || cmd == "h") {
            print_help(); 
            show_dur = false;
        }
        else {
            std::cout << "Unknown command: '" << cmd << "'. Try 'help'." << std::endl;
            show_dur = false;
        }

        // Affichage du temps d'exécution
        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)" << std::endl;
        }
    }

    if (db) kiva_close(db);
    std::cout << "Goodbye." << std::endl;
    
    return 0;
}