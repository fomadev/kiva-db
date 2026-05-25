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
 * RÈGLE v2.1.5 : Une clé est valide si elle n'est pas purement numérique.
 * Cela permet '2a' mais interdit '2', évitant les conflits avec le moteur mathématique.
 */
bool is_valid_key_name(const std::string& key) {
    if (key.empty()) return false;
    // Une clé est invalide UNIQUEMENT si elle ne contient que des chiffres.
    return !std::all_of(key.begin(), key.end(), ::isdigit);
}

/**
 * Vérifie si une chaîne de caractères est un nombre entier positif.
 */
bool is_number(const std::string& s) {
    return !s.empty() && std::find_if(s.begin(), s.end(), 
        [](unsigned char c) { return !std::isdigit(c); }) == s.end();
}

/**
 * Sépare les tokens en groupes basés sur le mot-clé "and".
 * Permet le chaînage de commandes comme 'get a and b'
 */
std::vector<std::vector<std::string>> split_by_and(const std::vector<std::string>& tokens) {
    std::vector<std::vector<std::string>> groups;
    std::vector<std::string> current;
    for (const auto& t : tokens) {
        if (t == "and") {
            if (!current.empty()) groups.push_back(current);
            current.clear();
        } else {
            current.push_back(t);
        }
    }
    if (!current.empty()) groups.push_back(current);
    return groups;
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

    std::cout << "KivaDB Shell v2.1.8 (Strict Mode with Chaining)\nType 'help' or 'h' for command list\n";

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

        // --- ROUTAGE VERSION 2.1.5 (STRICT ERROR MODE + CHAINING) ---

        if (cmd == "set") {
            auto groups = split_by_and(tokens);
            for (auto& segment : groups) {
                if (segment[0] != "set") segment.insert(segment.begin(), "set");
                size_t sn = segment.size();

                // Validation syntaxique + Validation de la clé (pas purement numérique)
                std::string key = (sn >= 3 && is_kiva_type(segment[1])) ? segment[2] : (sn >= 2 ? segment[1] : "");
                
                bool has_ttl_5 = (sn == 5 && segment[3] == "ttl" && is_number(segment[4]));
                bool has_ttl_6 = (sn == 6 && is_kiva_type(segment[1]) && segment[4] == "ttl" && is_number(segment[5]));
                
                bool ok = (sn == 3) || (sn == 4 && is_kiva_type(segment[1])) || has_ttl_5 || has_ttl_6;

                if (ok && is_valid_key_name(key)) {
                    handle_set(&db, segment, delimiters);
                } else if (ok && !is_valid_key_name(key)) {
                    std::cerr << "Error: InvalidKeyName: '" << key << "' cannot be purely numeric." << std::endl;
                    show_dur = false; break;
                } else {
                    std::cerr << "Error: Invalid set syntax near '" << segment.back() << "'\nUsage: set [type] <key> <value> [ttl <sec>]" << std::endl;
                    show_dur = false; break;
                }
            }
        }
        else if (cmd == "get") {
            auto groups = split_by_and(tokens);
            for (auto& segment : groups) {
                if (segment[0] != "get") segment.insert(segment.begin(), "get");
                size_t sn = segment.size();
                
                if ((sn == 2) || (sn == 3 && is_kiva_type(segment[1]))) {
                    handle_get(&db, segment, delimiters);
                } else {
                    std::cerr << "Error: Invalid get syntax near '" << segment.back() << "'\nUsage: get [type] <key>" << std::endl;
                    show_dur = false; break;
                }
            }
        }
        else if (cmd == "update") {
            auto groups = split_by_and(tokens);
            for (auto& segment : groups) {
                if (segment[0] != "update") segment.insert(segment.begin(), "update");
                size_t sn = segment.size();

                if ((sn == 3) || (sn == 4 && is_kiva_type(segment[1]))) {
                    handle_update(&db, segment, delimiters);
                } else {
                    std::cerr << "Error: Invalid update syntax near '" << segment.back() << "'\nUsage: update [type] <key> <value>" << std::endl;
                    show_dur = false; break;
                }
            }
        }
        else if (cmd == "del") {
            if (n == 3 && tokens[1] == "all" && tokens[2] == "keys") {
                KivaStatus status = kiva_reset(db);
                if (status == KIVA_OK) {
                    std::cout << "All keys deleted. Database reset." << std::endl;
                } else {
                    std::cerr << "Error: Could not reset database." << std::endl;
                }
            } 
            else {
                auto groups = split_by_and(tokens);
                for (auto& segment : groups) {
                    if (segment[0] != "del") segment.insert(segment.begin(), "del");
                    size_t sn = segment.size();
                    if ((sn == 2) || (sn == 3 && is_kiva_type(segment[1]))) {
                        handle_del(&db, segment, db_path);
                    } else {
                        std::cerr << "Error: Invalid del syntax near '" << segment.back() << "'" << std::endl;
                        show_dur = false; break;
                    }
                }
            }
        }
        else if (cmd == "typeof") {
            auto groups = split_by_and(tokens);
            for (auto& segment : groups) {
                if (segment[0] != "typeof") segment.insert(segment.begin(), "typeof");
                if (segment.size() == 2) {
                    handle_typeof(&db, segment);
                } else {
                    std::cerr << "Error: Usage: typeof <key> (near '" << segment.back() << "')" << std::endl;
                    show_dur = false; break;
                }
            }
        }
        else if (cmd == "change") {
            // est maintenant gérée exclusivement à l'intérieur de handle_change.
            
            if (n >= 4) {
                // On vérifie simplement la présence du mot-clé "to" pour un formatage minimal
                bool has_to = false;
                for (const auto& t : tokens) {
                    if (t == "to") {
                        has_to = true;
                        break;
                    }
                }

                if (has_to) {
                    handle_change(&db, tokens, delimiters);
                } else {
                    std::cerr << "Error: Missing 'to' keyword.\nUsage: change [type] <old> to [type] <new> [value]" << std::endl;
                    show_dur = false;
                }
            } else {
                std::cerr << "Error: Invalid change syntax.\nUsage: change [type] <old> to [type] <new> [value]" << std::endl;
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
            auto groups = split_by_and(tokens);
            for (auto& segment : groups) {
                if (segment[0] != "has") segment.insert(segment.begin(), "has");
                size_t sn = segment.size();
                if (sn == 2 || (sn == 3 && is_kiva_type(segment[1]))) {
                    handle_has(&db, segment);
                } else {
                    std::cerr << "Error: Usage: has [type] <key> (near '" << segment.back() << "')" << std::endl;
                    show_dur = false; break;
                }
            }
        }
        else if (cmd == "print") {
            handle_print(&db, tokens, delimiters);
        }
        else if (cmd == "bump") {
            handle_bump(&db, tokens);
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