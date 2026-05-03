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
 * Utilitaire pour le parsing des commandes
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
};

void print_help() {
    std::cout << "\n--- KivaDB Shell Help (C++ Engine) ---\n"
              << "  set [type] <key> <val> [ttl <sec>] : Create pair(s) with optional TTL\n"
              << "                                       Types: string, number, boolean\n"
              << "                                       Ex: set user \"Alex\" ttl 60\n"
              << "  update <key> <val>                 : Update EXISTING key(s)\n"
              << "  get <key>                          : Retrieve value\n"
              << "  typeof <key>                       : Show the dynamic data type\n"
              << "  del <key>                          : Remove key(s)\n"
              << "  scan                               : List all keys (shows TTL if active)\n"
              << "  stats                              : Database health & file size\n"
              << "  compact                            : Reclaim disk space & clean expired keys\n"
              << "  exit                               : Quit\n"
              << "-------------------------\n";
}

int main(int argc, char* argv[]) {
    if (argc > 1) {
        std::string arg = argv[1];
        if (arg == "-v" || arg == "--version") {
            std::cout << "kivadb version " << KIVADB_VERSION << std::endl;
            return 0;
        }
    }

    // Utilisation de la macro MKDIR définie dans kivadb.h
    MKDIR("data");
    const char* db_path = "data/store.kiva";
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Erreur : Impossible d'ouvrir " << db_path << std::endl;
        return 1;
    }

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

        if (cmd == "set") {
            int created = 0;
            int global_ttl = 0;
            
            // Extraction du TTL
            for (size_t j = 0; j < tokens.size(); j++) {
                if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
                    try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
                    break;
                }
            }

            for (size_t i = 1; i < tokens.size(); ) {
                if (tokens[i] == "and" || tokens[i] == "ttl") { 
                    if(tokens[i] == "ttl") i += 2; else i++;
                    continue; 
                }

                KivaType forced = KIVA_TYPE_UNKNOWN;
                if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
                else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
                else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

                if (i + 1 >= tokens.size()) break;

                std::string key = tokens[i++];
                std::string val = tokens[i++];

                kiva_set_ex(db, key.c_str(), val.c_str(), forced, global_ttl);
                std::cout << "OK: " << key << " saved.\n";
                created++;
            }
            std::cout << "Summary: " << created << " key(s) processed.";
        }
        else if (cmd == "get") {
            if (tokens.size() < 2) continue;
            char* res = kiva_get(db, tokens[1].c_str());
            std::cout << tokens[1] << ": " << (res ? res : "(nil)");
            if (res) free(res);
        }
        else if (cmd == "scan") {
            index_scan(db);
            show_duration = false;
        }
        else if (cmd == "compact") {
            kiva_compact(db);
            std::cout << "Compaction and cleaning done.";
        }
        else if (cmd == "stats") {
            std::cout << "\n--- Stats ---\n"
                      << "Keys: " << index_get_count(db) << "\n"
                      << "File: " << kiva_get_file_size(db_path) << " bytes\n";
            show_duration = false;
        }
        else if (cmd == "help" || cmd == "h") {
            print_help();
            show_duration = false;
        }
        else if (cmd == "del") {
            if (tokens.size() >= 2) {
                kiva_delete(db, tokens[1].c_str());
                std::cout << "OK.";
            }
        }
        else {
            std::cout << "Unknown command. Type 'help'.";
            show_duration = false;
        }

        clock_t end = clock();
        if (show_duration) {
            double duration = static_cast<double>(end - start) / CLOCKS_PER_SEC;
            std::cout << " (" << std::fixed << std::setprecision(6) << duration << "s)\n";
        } else {
            std::cout << "\n";
        }
    }

    kiva_close(db);
    return 0;
}