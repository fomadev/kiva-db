#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>
#include <algorithm>

// On inclut d'abord les headers du projet. 
extern "C" {
    #include "../../include/kivadb.h"
    #include "../core/kivadb_internal.h"
}

// Sécurité supplémentaire : on ne définit MKDIR que s'il n'existe pas encore
#ifndef MKDIR
    #ifdef _WIN32
        #include <direct.h>
        #define MKDIR(path) _mkdir(path)
    #else
        #include <sys/stat.h>
        #define MKDIR(path) mkdir(path, 0777)
    #endif
#endif

/**
 * CommandParser : Identifie les jetons et leurs délimiteurs respectifs
 */
struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input, std::vector<char>& delimiters) {
        std::vector<std::string> tokens;
        std::string current;
        char quote_char = 0;

        for (size_t i = 0; i < input.length(); ++i) {
            char c = input[i];

            // Début d'une zone citée
            if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
                quote_char = c;
            } 
            // Fin d'une zone citée
            else if (c == quote_char && quote_char != 0) {
                tokens.push_back(current);
                delimiters.push_back(quote_char);
                current.clear();
                quote_char = 0;
            } 
            // Espace hors guillemets (séparateur)
            else if (isspace(c) && quote_char == 0) {
                if (!current.empty()) {
                    tokens.push_back(current);
                    delimiters.push_back(0); // 0 = Aucun délimiteur
                    current.clear();
                }
            } 
            // Caractère standard
            else {
                current += c;
            }
        }

        if (quote_char != 0) {
            std::cout << "Syntax Error: Unclosed quote detected (" << quote_char << ").\n";
            delimiters.clear();
            return {};
        }

        if (!current.empty()) {
            tokens.push_back(current);
            delimiters.push_back(0);
        }
        return tokens;
    }
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v2.0.0) ---\n"
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

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    MKDIR("data");
    const char* db_path = "data/store.kiva";
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open database." << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v2.0.0\nType 'help' for commands\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        std::vector<char> delimiters;
        auto tokens = CommandParser::tokenize(line, delimiters);
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_dur = true;

        if (cmd == "set") {
            int global_ttl = 0;
            // Recherche du TTL dans toute la ligne
            for (size_t j = 0; j < tokens.size(); j++) {
                if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
                    try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
                }
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

                // --- VALIDATION STRICTE ---
                // La Clé (i) : Pas de " ou '
                if (delimiters[i] == '"' || delimiters[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' cannot use \"\" or ''. Use backticks `` or nothing.\n";
                    i += 2; continue;
                }
                // La Valeur (i+1) : Obligatoirement " ou '
                if (delimiters[i+1] != '"' && delimiters[i+1] != '\'') {
                    std::cout << "Error: Value for '" << tokens[i] << "' must be enclosed in \"\" or ''.\n";
                    i += 2; continue;
                }

                char* check_exists = kiva_get(db, tokens[i].c_str());
                if (check_exists) {
                    std::cout << "Error: Key '" << tokens[i] << "' already exists. Use 'update'.\n";
                    free(check_exists);
                    i += 2; continue;
                }

                kiva_set_ex(db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);
                std::cout << "OK: " << tokens[i] << " saved.\n";
                i += 2;
            }
        }
        else if (cmd == "update") {
            for (size_t i = 1; i + 1 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }

                // Validation Clé/Valeur
                if (delimiters[i] == '"' || delimiters[i] == '\'') {
                    std::cout << "Error: Key cannot use quotes.\n"; i += 2; continue;
                }
                if (delimiters[i+1] != '"' && delimiters[i+1] != '\'') {
                    std::cout << "Error: Value must be quoted.\n"; i += 2; continue;
                }

                char* check = kiva_get(db, tokens[i].c_str());
                if (!check) { 
                    std::cout << "Error: " << tokens[i] << " not found. Use 'set'.\n"; 
                } else {
                    kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
                    std::cout << "OK: " << tokens[i] << " updated.\n";
                    free(check);
                }
                i += 2;
            }
        }
        else if (cmd == "get") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                
                // Règle : Une clé demandée en GET ne doit pas être entre ""
                if (delimiters[i] == '"' || delimiters[i] == '\'') {
                    std::cout << "Error: Key '" << tokens[i] << "' is quoted. Keys in KivaDB are bare or in ``.\n";
                    continue;
                }

                char* res = kiva_get(db, tokens[i].c_str());
                std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
                if (res) free(res);
            }
        }
        else if (cmd == "change") {
            for (size_t i = 1; i + 2 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                if (tokens[i+1] == "to") {
                    char* val = kiva_get(db, tokens[i].c_str());
                    if (!val) {
                        std::cout << "Error: Source '" << tokens[i] << "' not found.\n";
                        i += 3; continue;
                    }
                    char* target_exists = kiva_get(db, tokens[i+2].c_str());
                    if (target_exists) {
                        std::cout << "Error: Target '" << tokens[i+2] << "' already exists.\n";
                        free(val); free(target_exists);
                        i += 3; continue;
                    }
                    kiva_set(db, tokens[i+2].c_str(), val); 
                    kiva_delete(db, tokens[i].c_str());
                    std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
                    free(val);
                    i += 3;
                } else i++;
            }
        }
        else if (cmd == "typeof") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                const char* t_name = kiva_typeof(db, tokens[i].c_str());
                std::cout << " -> " << tokens[i] << " is a [" << t_name << "]\n";
            }
        }
        else if (cmd == "del") {
            if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
                kiva_close(db); 
                remove(db_path);
                db = kiva_open(db_path);
                std::cout << "All keys cleared.\n";
            } else {
                for (size_t i = 1; i < tokens.size(); i++) {
                    if (tokens[i] == "and") continue;
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
        else if (cmd == "compact") { kiva_compact(db); std::cout << "Database compacted.\n"; }
        else if (cmd == "help" || cmd == "h") { print_help(); show_dur = false; }
        else { std::cout << "Unknown command.\n"; show_dur = false; }

        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    kiva_close(db);
    return 0;
}