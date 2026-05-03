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
 * CommandParser avancé : Gère les "", '', et ``
 * Remplit le vecteur delimiters pour savoir quel symbole entourait chaque token.
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
        if (!current.empty()) {
            tokens.push_back(current);
            delimiters.push_back(0);
        }
        return tokens;
    }
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (v1.1.0) ---\n"
              << "  set [type] `key` \"val\" [ttl s]   : Set with forced quotes for strings\n"
              << "  update `key` \"val\"               : Update existing keys (supports 'and')\n"
              << "  change `old` to `new`            : Rename keys (supports 'and')\n"
              << "  get `key1` and `key2`            : Retrieve values\n"
              << "  typeof `key`                     : Show data type (supports 'and')\n"
              << "  del `key` OR del all keys        : Delete keys\n"
              << "  scan                             : List all entries\n"
              << "  compact | stats | exit           : Utility commands\n"
              << "-----------------------------------\n";
}

int main(int argc, char* argv[]) {
    // Élimination des warnings de compilation
    (void)argc; 
    (void)argv;

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
                    i += (tokens[i] == "ttl" ? 2 : 1); 
                    continue; 
                }
                
                KivaType forced = KIVA_TYPE_UNKNOWN;
                if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
                else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
                else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

                if (i + 1 >= tokens.size()) break;

                std::string key = tokens[i];
                std::string val = tokens[i+1];
                
                // Rigueur : String doit être entre "" ou ''
                if ((forced == KIVA_TYPE_STRING || forced == KIVA_TYPE_UNKNOWN) && 
                    (delim[i+1] != '"' && delim[i+1] != '\'')) {
                    std::cout << "Syntax Error: Value for '" << key << "' must be in \"\" or ''.\n";
                    i += 2; continue;
                }

                kiva_set_ex(db, key.c_str(), val.c_str(), forced, global_ttl);
                std::cout << "OK: " << key << " saved.\n";
                i += 2;
            }
        }
        // --- COMMAND: UPDATE ---
        else if (cmd == "update") {
            for (size_t i = 1; i + 1 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                char* check = kiva_get(db, tokens[i].c_str());
                if (!check) { 
                    std::cout << "Error: " << tokens[i] << " not found. Use 'set' to create.\n"; 
                } else {
                    kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
                    std::cout << "OK: " << tokens[i] << " updated.\n";
                    free(check);
                }
                i += 2;
            }
        }
        // --- COMMAND: CHANGE (RENAME) ---
        else if (cmd == "change") {
            for (size_t i = 1; i + 2 < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                if (tokens[i+1] == "to") {
                    char* val = kiva_get(db, tokens[i].c_str());
                    if (val) {
                        kiva_set(db, tokens[i+2].c_str(), val);
                        kiva_delete(db, tokens[i].c_str());
                        std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
                        free(val);
                    } else { 
                        std::cout << "Error: Source key '" << tokens[i] << "' not found.\n"; 
                    }
                    i += 3;
                } else i++;
            }
        }
        // --- COMMAND: GET ---
        else if (cmd == "get") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                char* res = kiva_get(db, tokens[i].c_str());
                std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
                if (res) free(res);
            }
        }
        // --- COMMAND: TYPEOF ---
        else if (cmd == "typeof") {
            for (size_t i = 1; i < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                const char* t_name = kiva_typeof(db, tokens[i].c_str());
                std::cout << " -> " << tokens[i] << " is a [" << t_name << "]\n";
            }
        }
        // --- COMMAND: DEL ---
        else if (cmd == "del") {
            if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
                kiva_close(db);
                remove(db_path);
                db = kiva_open(db_path);
                std::cout << "All keys cleared from disk successfully.\n";
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
        else if (cmd == "compact") { 
            kiva_compact(db); 
            std::cout << "Compaction and TTL cleaning done.\n"; 
        }
        else if (cmd == "help" || cmd == "h") { 
            print_help(); 
            show_dur = false; 
        }
        else { 
            std::cout << "Unknown command: " << cmd << ". Type 'help' for info.\n"; 
            show_dur = false; 
        }

        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    kiva_close(db);
    return 0;
}