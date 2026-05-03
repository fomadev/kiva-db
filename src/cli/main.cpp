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
 * Tokenizer intelligent :
 * - `key` -> traité comme une clé
 * - "val" ou 'val' -> traité comme une valeur string
 */
struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input) {
        std::vector<std::string> tokens;
        std::string current;
        char quote_char = 0;

        for (size_t i = 0; i < input.length(); ++i) {
            char c = input[i];
            if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
                quote_char = c;
            } else if (c == quote_char) {
                tokens.push_back(current);
                current.clear();
                quote_char = 0;
            } else if (isspace(c) && quote_char == 0) {
                if (!current.empty()) {
                    tokens.push_back(current);
                    current.clear();
                }
            } else {
                current += c;
            }
        }
        if (!current.empty()) tokens.push_back(current);
        return tokens;
    }

    // Vérifie si une valeur dans la ligne de commande était entourée de guillemets
    static bool was_quoted(const std::string& line, const std::string& token) {
        size_t pos = line.find(token);
        if (pos == std::string::npos || pos == 0) return false;
        char before = line[pos - 1];
        return (before == '"' || before == '\'');
    }
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (FomaDev Edition) ---\n"
              << "  set [type] <key> <val> [ttl <sec>] : Create pair. Strings MUST use \"\" or ''\n"
              << "  update <key> <val>                 : Update existing key(s)\n"
              << "  change <old> to <new>              : Rename a key\n"
              << "  get <key1> and <key2>              : Retrieve multiple values\n"
              << "  typeof <key1> and <key2>           : Show data types\n"
              << "  del <key1> and <key2>              : Remove multiple keys\n"
              << "  del all keys                       : Clear entire database\n"
              << "  scan | stats | compact | exit      : General commands\n"
              << "-------------------------------------------\n";
}

int main(int argc, char* argv[]) {
    MKDIR("data");
    const char* db_path = "data/store.kiva";
    KivaDB* db = kiva_open(db_path);
    if (!db) return 1;

    std::cout << "KivaDB Shell v" << KIVADB_VERSION << " (C++ Engine Ready)\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        auto tokens = CommandParser::tokenize(line);
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_duration = true;

        // --- COMMAND: SET ---
        if (cmd == "set") {
            int global_ttl = 0;
            for (size_t j = 0; j < tokens.size(); j++) {
                if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
                    global_ttl = std::stoi(tokens[j+1]);
                    break;
                }
            }

            for (size_t i = 1; i < tokens.size(); ) {
                if (tokens[i] == "and" || tokens[i] == "ttl") { 
                    i += (tokens[i] == "ttl") ? 2 : 1; continue; 
                }

                KivaType forced = KIVA_TYPE_UNKNOWN;
                if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
                else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
                else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

                if (i + 1 >= tokens.size()) break;
                std::string key = tokens[i++];
                std::string val = tokens[i++];

                // Rigueur : Si c'est un string (ou auto-détecté comme tel), il faut des quotes
                if (!CommandParser::was_quoted(line, val) && forced != KIVA_TYPE_NUMBER && forced != KIVA_TYPE_BOOLEAN) {
                    std::cout << "Syntax Error: String values must be enclosed in \"\" or ''\n";
                    continue;
                }

                kiva_set_ex(db, key.c_str(), val.c_str(), forced, global_ttl);
                std::cout << "OK: " << key << " saved.\n";
            }
        }
        // --- COMMAND: GET / TYPEOF / DEL ---
        else if (cmd == "get" || cmd == "typeof" || cmd == "del") {
            if (cmd == "del" && tokens.size() >= 3 && tokens[1] == "all" && tokens[2] == "keys") {
                kiva_close(db);
                remove(db_path);
                db = kiva_open(db_path);
                std::cout << "Database wiped. (All keys deleted)";
            } else {
                for (size_t i = 1; i < tokens.size(); ++i) {
                    if (tokens[i] == "and") continue;
                    if (cmd == "get") {
                        char* res = kiva_get(db, tokens[i].c_str());
                        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
                        if (res) free(res);
                    } else if (cmd == "typeof") {
                        std::cout << tokens[i] << ": " << kiva_typeof(db, tokens[i].c_str()) << "\n";
                    } else if (cmd == "del") {
                        if (kiva_delete(db, tokens[i].c_str()) == KIVA_OK) std::cout << "Deleted: " << tokens[i] << "\n";
                        else std::cout << "Not found: " << tokens[i] << "\n";
                    }
                }
            }
        }
        // --- COMMAND: CHANGE (Rename) ---
        else if (cmd == "change") {
            for (size_t i = 1; i + 2 < tokens.size(); i++) {
                if (tokens[i] == "and") continue;
                std::string old_k = tokens[i];
                if (tokens[i+1] == "to") {
                    std::string new_k = tokens[i+2];
                    char* val = kiva_get(db, old_k.c_str());
                    if (val) {
                        kiva_set(db, new_k.c_str(), val);
                        kiva_delete(db, old_k.c_str());
                        free(val);
                        std::cout << "Changed: " << old_k << " -> " << new_k << "\n";
                    } else { std::cout << "Error: " << old_k << " not found.\n"; }
                    i += 2;
                }
            }
        }
        // --- COMMAND: UPDATE ---
        else if (cmd == "update") {
            for (size_t i = 1; i + 1 < tokens.size(); i += 2) {
                if (tokens[i] == "and") { i--; i += 2; continue; }
                char* check = kiva_get(db, tokens[i].c_str());
                if (check) {
                    kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
                    std::cout << "Updated: " << tokens[i] << "\n";
                    free(check);
                } else { std::cout << "Error: " << tokens[i] << " doesn't exist. Use 'set'.\n"; }
            }
        }
        else if (cmd == "scan") { index_scan(db); show_duration = false; }
        else if (cmd == "stats") { 
            std::cout << "Keys: " << index_get_count(db) << " | File: " << kiva_get_file_size(db_path) << " bytes\n"; 
            show_duration = false; 
        }
        else if (cmd == "compact") { kiva_compact(db); std::cout << "Compacted."; }
        else if (cmd == "help" || cmd == "h") { print_help(); show_duration = false; }
        else { std::cout << "Unknown command: " << cmd; show_duration = false; }

        if (show_duration) {
            double duration = static_cast<double>(clock() - start) / CLOCKS_PER_SEC;
            std::cout << " (" << std::fixed << std::setprecision(6) << duration << "s)\n";
        } else std::cout << "\n";
    }
    kiva_close(db);
    return 0;
}