#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>
#include <algorithm>

#ifdef _WIN32
    #include <direct.h>
    #define MKDIR(d) _mkdir(d)
#else
    #include <sys/stat.h>
    #define MKDIR(d) mkdir(d, 0777)
#endif

extern "C" {
    #include "../../include/kivadb.h"
    #include "../core/kivadb_internal.h"
}

/**
 * Utilitaire pour le parsing des commandes et la détection de types
 */
struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input) {
        std::vector<std::string> tokens;
        std::string current;
        bool in_quotes = false;

        for (char c : input) {
            if (c == '"') {
                in_quotes = !in_quotes;
            } else if (c == ' ' && !in_quotes) {
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

    static bool is_number(const std::string& s) {
        if (s.empty()) return false;
        char* p;
        strtod(s.c_str(), &p);
        return *p == 0;
    }
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (C++ Engine) ---\n"
              << "  set [type] <key> <val> : Create NEW key-value pair(s)\n"
              << "                           Types: string, number, boolean (optional)\n"
              << "                           Multiple: set k1 v1 and k2 v2\n"
              << "  update <key> <val>     : Update EXISTING key(s)\n"
              << "  get <key>              : Retrieve value of one or more keys\n"
              << "  typeof <key>           : Show the dynamic data type\n"
              << "  del <key>              : Remove one or more keys\n"
              << "  scan                   : List all keys with their types and sizes\n"
              << "  stats                  : Show database health and file size\n"
              << "  compact                : Reclaim disk space (defragmentation)\n"
              << "  help or h              : Show this help menu\n"
              << "  exit                   : Close database and quit\n"
              << "-------------------------\n";
}

int main(int argc, char* argv[]) {
    if (argc > 1) {
        std::string arg = argv[1];
        if (arg == "-version" || arg == "--version" || arg == "-v") {
            std::cout << "kivadb version " << KIVADB_VERSION << std::endl;
            return 0;
        }
        if (arg == "--help" || arg == "-h") {
            print_help();
            return 0;
        }
    }

    MKDIR("data");
    const char* db_path = "data/store.kiva";
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Erreur : Impossible d'ouvrir ou créer " << db_path << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v" << KIVADB_VERSION << " (C++ Engine Ready)\n"
              << "Type 'help' for commands" << std::endl;

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

        if (cmd == "set") {
            int created = 0;
            for (size_t i = 1; i < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }

                KivaType forced = KIVA_TYPE_UNKNOWN;
                if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
                else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
                else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

                if (i + 1 >= tokens.size()) {
                    std::cout << "Error: Missing key or value." << std::endl;
                    break;
                }

                std::string key = tokens[i++];
                std::string val = tokens[i++];

                // On vérifie si la clé existe déjà
                char* existing = kiva_get(db, key.c_str());
                if (existing) {
                    std::cout << "Error: Key '" << key << "' already exists. Use 'update'." << std::endl;
                    free(existing);
                    continue;
                }

                kiva_set_with_type(db, key.c_str(), val.c_str(), forced);
                std::cout << "OK: " << key << " saved." << std::endl;
                created++;
            }
            std::cout << "Summary: " << created << " key(s) created.";
        }
        else if (cmd == "update") {
            int updated = 0;
            for (size_t i = 1; i < tokens.size(); ) {
                if (tokens[i] == "and") { i++; continue; }
                if (i + 1 >= tokens.size()) break;

                std::string key = tokens[i++];
                std::string val = tokens[i++];

                char* existing = kiva_get(db, key.c_str());
                if (!existing) {
                    std::cout << "Error: Key '" << key << "' not found." << std::endl;
                } else {
                    kiva_set(db, key.c_str(), val.c_str());
                    std::cout << "OK: " << key << " updated." << std::endl;
                    updated++;
                    free(existing);
                }
            }
            std::cout << "Summary: " << updated << " key(s) updated.";
        }
        else if (cmd == "get" || cmd == "del" || cmd == "typeof") {
            for (size_t i = 1; i < tokens.size(); ++i) {
                if (tokens[i] == "and") continue;
                
                if (cmd == "get") {
                    char* res = kiva_get(db, tokens[i].c_str());
                    std::cout << tokens[i] << ": " << (res ? res : "(nil)") << std::endl;
                    if (res) free(res);
                } else if (cmd == "del") {
                    if (kiva_delete(db, tokens[i].c_str()) == KIVA_OK) 
                        std::cout << "Deleted: " << tokens[i] << std::endl;
                    else 
                        std::cout << "Error: " << tokens[i] << " not found." << std::endl;
                } else if (cmd == "typeof") {
                    std::cout << tokens[i] << ": " << kiva_typeof(db, tokens[i].c_str()) << std::endl;
                }
            }
        }
        else if (cmd == "scan") {
            index_scan(db);
            show_duration = false;
        }
        else if (cmd == "compact") {
            kiva_compact(db);
            std::cout << "Compaction successful.";
        }
        else if (cmd == "stats") {
            std::cout << "\n--- Stats ---\n"
                      << "Keys: " << index_get_count(db) << "\n"
                      << "File: " << kiva_get_file_size(db_path) << " bytes\n"
                      << "-------------";
            show_duration = false;
        }
        else if (cmd == "help" || cmd == "h") {
            print_help();
            show_duration = false;
        }
        else {
            std::cout << "Unknown command: " << cmd;
            show_duration = false;
        }

        clock_t end = clock();
        if (show_duration) {
            double duration = static_cast<double>(end - start) / CLOCKS_PER_SEC;
            std::cout << " (" << std::fixed << std::setprecision(6) << duration << " sec)\n";
        } else {
            std::cout << "\n";
        }
    }

    kiva_close(db);
    std::cout << "Bye!" << std::endl;
    return 0;
}