/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>
#include <algorithm>

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

/**
 * Vérifie si une chaîne de caractères est un nombre entier positif.
 */
bool is_number(const std::string& s) {
    return !s.empty() && std::find_if(s.begin(), s.end(), 
        [](unsigned char c) { return !std::isdigit(c); }) == s.end();
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

    std::cout << "KivaDB Shell v2.1.2 (Strict Mode)\nType 'help' or 'h' for command list\n";

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

        // --- ROUTAGE VERSION 2.1.2 (STRICT ERROR MODE) ---

        if (cmd == "set") {
            // Analyse des jetons pour valider le TTL si présent
            // Format 5: set <k> <v> ttl <sec>
            bool has_ttl_5 = (n == 5 && tokens[3] == "ttl" && is_number(tokens[4]));
            // Format 6: set <t> <k> <v> ttl <sec>
            bool has_ttl_6 = (n == 6 && is_kiva_type(tokens[1]) && tokens[4] == "ttl" && is_number(tokens[5]));
            
            bool ok = (n == 3) || 
                      (n == 4 && is_kiva_type(tokens[1])) || 
                      has_ttl_5 || 
                      has_ttl_6;

            if (ok) {
                handle_set(&db, tokens, delimiters);
            } else {
                // Message d'erreur spécifique pour le TTL mal formé
                if ((n == 5 && tokens[3] == "ttl") || (n == 6 && tokens[4] == "ttl")) {
                    std::cerr << "Error: TTL must be a positive number." << std::endl;
                } else {
                    std::cerr << "Error: Invalid set syntax.\nUsage: set [type] <key> <value> [ttl <sec>]" << std::endl;
                }
                show_dur = false;
            }
        }
        else if (cmd == "get") {
            bool ok = (n == 2) || (n == 3 && is_kiva_type(tokens[1]));
            if (ok) {
                handle_get(&db, tokens, delimiters);
            } else {
                std::cerr << "Error: Invalid get syntax.\nUsage: get [type] <key>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "update") {
            bool ok = (n == 3) || (n == 4 && is_kiva_type(tokens[1]));
            if (ok) {
                handle_update(&db, tokens, delimiters);
            } else {
                std::cerr << "Error: Invalid update syntax.\nUsage: update [type] <key> <value>" << std::endl;
                show_dur = false;
            }
        }
        else if (cmd == "del") {
            bool is_reset = (n == 3 && tokens[1] == "all" && tokens[2] == "keys");
            bool is_standard = (n == 2) || (n == 3 && is_kiva_type(tokens[1]));

            if (is_reset) {
                KivaStatus status = kiva_reset(db);
                if (status == KIVA_OK) {
                    std::cout << "All keys deleted. Database reset." << std::endl;
                } else {
                    std::cerr << "Error: Could not reset database." << std::endl;
                }
            } 
            else if (is_standard) {
                handle_del(&db, tokens, db_path);
            } 
            else {
                std::cerr << "Error: Invalid del syntax.\nUsage: del [type] <key> OR del all keys" << std::endl;
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
        else if (cmd == "has") {
            if (n == 2 || (n == 3 && is_kiva_type(tokens[1]))) {
                handle_has(&db, tokens);
            } else {
                std::cerr << "Error: Usage: has [type] <key>" << std::endl;
                show_dur = false;
            }
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