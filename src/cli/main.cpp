#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>
#include <algorithm>

extern "C" {
    #include "../../include/kivadb.h"
    #include "../core/kivadb_internal.h"
}

/**
 * CommandParser strict : Gère les "", '', et ``
 * Vérifie maintenant si les guillemets sont correctement fermés.
 */
struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input, std::vector<char>& delimiters) {
        std::vector<std::string> tokens;
        std::string current;
        char quote_char = 0;

        for (char c : input) {
            if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
                quote_char = c;
            } else if (c == quote_char) {
                tokens.push_back(current);
                delimiters.push_back(quote_char);
                current.clear();
                quote_char = 0;
            } else if (isspace(c) && quote_char == 0) {
                if (!current.empty()) {
                    tokens.push_back(current);
                    delimiters.push_back(0); 
                    current.clear();
                }
            } else {
                current += c;
            }
        }

        // --- CORRECTION : Détection des guillemets non fermés ---
        if (quote_char != 0) {
            std::cout << "Syntax Error: Unclosed quote detected (" << quote_char << ").\n";
            delimiters.clear();
            return {}; // Retourne un vecteur vide pour annuler la commande
        }

        if (!current.empty()) {
            tokens.push_back(current);
            delimiters.push_back(0);
        }
        return tokens;
    }
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v1.1.0) ---\n"
              << "  set [type] `key` \"val\" [ttl s]   : Set with strict quotes\n"
              << "  update `key` \"val\"               : Update existing keys\n"
              << "  change `old` to `new`            : Rename and remove TTL\n"
              << "  get `key1` and `key2`            : Retrieve values\n"
              << "  typeof `key`                     : Show data type\n"
              << "  del `key` OR del all keys        : Delete keys\n"
              << "  scan                             : List all entries\n"
              << "  compact | stats | exit           : Utility commands\n"
              << "-----------------------------------\n";
}

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    MKDIR("data");
    const char* db_path = "data/store.kiva";
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open database." << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell (C++ Engine Ready)\nType 'help' for commands\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        std::vector<char> delim;
        auto tokens = CommandParser::tokenize(line, delim);
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_dur = true;

        // --- COMMAND: SET ---
        if (cmd == "set") {
            int global_ttl = 0;
            for (size_t j = 0; j < tokens.size(); j++) 
                if (tokens[j] == "ttl" && j+1 < tokens.size()) {
                    try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
                }

            for (size_t i = 1; i < tokens.size(); ) {
                if (tokens[i] == "and" || tokens[i] == "ttl") { 
                    i += (tokens[i] == "ttl" ? 2 : 1); continue; 
                }
                
                KivaType forced = KIVA_TYPE_UNKNOWN;
                if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
                else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
                else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

                if (i + 1 >= tokens.size()) break;

                // Rigueur : Clé ne peut pas avoir "" ou ''
                if (delim[i] == '"' || delim[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''. Use ``.\n";
                    i += 2; continue;
                }

                // Rigueur : Valeur doit avoir "" ou '' si c'est un string
                if ((forced == KIVA_TYPE_STRING || forced == KIVA_TYPE_UNKNOWN) && 
                    (delim[i+1] != '"' && delim[i+1] != '\'')) {
                    // On laisse passer les nombres/bool sans guillemets, mais pas le reste
                    if (forced == KIVA_TYPE_STRING) {
                        std::cout << "Syntax Error: String values must be in \"\" or ''.\n";
                        i += 2; continue;
                    }
                }

                kiva_set_ex(db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);
                std::cout << "OK: " << tokens[i] << " saved.\n";
                i += 2;
            }
        }
        // --- COMMAND: UPDATE ---
        else if (cmd == "update") {
            for (size_t i = 1; i + 1 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                if (delim[i] == '"' || delim[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''.\n";
                    i += 2; continue;
                }
                char* check = kiva_get(db, tokens[i].c_str());
                if (!check) { 
                    std::cout << "Error: " << tokens[i] << " not found.\n"; 
                } else {
                    kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
                    std::cout << "OK: " << tokens[i] << " updated.\n";
                    free(check);
                }
                i += 2;
            }
        }
        // --- COMMAND: CHANGE ---
        else if (cmd == "change") {
            for (size_t i = 1; i + 2 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                if (tokens[i+1] == "to") {
                    if (delim[i] == '"' || delim[i] == '\'' || delim[i+2] == '"' || delim[i+2] == '\'') {
                        std::cout << "Error: Keys cannot use \"\" or ''.\n";
                        i += 3; continue;
                    }
                    char* val = kiva_get(db, tokens[i].c_str());
                    if (val) {
                        kiva_set(db, tokens[i+2].c_str(), val); // Nouveau set = suppression TTL
                        kiva_delete(db, tokens[i].c_str());
                        std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
                        free(val);
                    } else { 
                        std::cout << "Error: '" << tokens[i] << "' not found.\n"; 
                    }
                    i += 3;
                } else i++;
            }
        }
        // --- COMMAND: GET ---
        else if (cmd == "get") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                if (delim[i] == '"' || delim[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''.\n";
                    continue;
                }
                char* res = kiva_get(db, tokens[i].c_str());
                std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
                if (res) free(res);
            }
        }
        // --- COMMAND: TYPEOF ---
        else if (cmd == "typeof") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                if (delim[i] == '"' || delim[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''.\n";
                    continue;
                }
                const char* t_name = kiva_typeof(db, tokens[i].c_str());
                std::cout << " -> " << tokens[i] << " is a [" << t_name << "]\n";
            }
        }
        // --- COMMAND: DEL ---
        else if (cmd == "del") {
            if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
                kiva_close(db); remove(db_path);
                db = kiva_open(db_path);
                std::cout << "All keys cleared.\n";
            } else {
                for (size_t i = 1; i < tokens.size(); i++) {
                    if (tokens[i] == "and") continue;
                    if (delim[i] == '"' || delim[i] == '\'') {
                        std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''.\n";
                        continue;
                    }
                    if (kiva_delete(db, tokens[i].c_str()) == KIVA_OK)
                        std::cout << "Deleted: " << tokens[i] << "\n";
                    else
                        std::cout << "Not found: " << tokens[i] << "\n";
                }
            }
        }
        else if (cmd == "scan") { index_scan(db); show_dur = false; }
        else if (cmd == "stats") {
            std::cout << "Keys: " << index_get_count(db) << " | File: " << kiva_get_file_size(db_path) << " bytes\n";
            show_dur = false;
        }
        else if (cmd == "compact") { kiva_compact(db); std::cout << "Done.\n"; }
        else if (cmd == "help" || cmd == "h") { print_help(); show_dur = false; }
        else { std::cout << "Unknown command. Type 'help'.\n"; show_dur = false; }

        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    kiva_close(db);
    return 0;
}