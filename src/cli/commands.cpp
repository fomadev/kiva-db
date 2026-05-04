#include "commands.hpp"
#include <iostream>
#include <cstdlib>

void handle_set(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
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

        if (delimiters[i] == '"' || delimiters[i] == '\'') {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes.\n";
            i += 2; continue;
        }
        if (delimiters[i+1] != '"' && delimiters[i+1] != '\'') {
            std::cout << "Error: Value for '" << tokens[i] << "' must be quoted.\n";
            i += 2; continue;
        }

        char* exists = kiva_get(db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' exists. Use 'update'.\n";
            free(exists); i += 2; continue;
        }

        kiva_set_ex(db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);
        std::cout << "OK: " << tokens[i] << " saved.\n";
        i += 2;
    }
}

void handle_get(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        if (delimiters[i] == '"' || delimiters[i] == '\'') {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted. Use bare text or ``.\n";
            continue;
        }
        char* res = kiva_get(db, tokens[i].c_str());
        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
        if (res) free(res);
    }
}

void handle_update(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }
        if (delimiters[i] == '"' || delimiters[i] == '\'' || (delimiters[i+1] != '"' && delimiters[i+1] != '\'')) {
            std::cout << "Error: Syntax error in update.\n"; i += 2; continue;
        }
        char* check = kiva_get(db, tokens[i].c_str());
        if (!check) std::cout << "Error: " << tokens[i] << " not found.\n";
        else {
            kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
            std::cout << "OK: " << tokens[i] << " updated.\n";
            free(check);
        }
        i += 2;
    }
}

void handle_change(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    (void)delimiters; 

    for (size_t i = 1; i + 2 < tokens.size(); ) {
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        if (tokens[i+1] == "to") {
            if (delimiters[i] == '"' || delimiters[i] == '\'' || 
                delimiters[i+2] == '"' || delimiters[i+2] == '\'') {
                std::cout << "Error: Keys cannot be enclosed in quotes for rename operation.\n";
                i += 3;
                continue;
            }

            char* val = kiva_get(db, tokens[i].c_str());
            if (!val) { 
                std::cout << "Error: Source key '" << tokens[i] << "' not found.\n"; 
                i += 3; 
                continue; 
            }

            char* target = kiva_get(db, tokens[i+2].c_str());
            if (target) { 
                std::cout << "Error: Target key '" << tokens[i+2] << "' already exists.\n"; 
                free(val); 
                free(target); 
                i += 3; 
                continue; 
            }

            kiva_set(db, tokens[i+2].c_str(), val); 
            kiva_delete(db, tokens[i].c_str());

            std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
            
            free(val); 
            i += 3;
        } else {
            i++;
        }
    }
}

void handle_typeof(KivaDB* db, const std::vector<std::string>& tokens) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        std::cout << " -> " << tokens[i] << " is a [" << kiva_typeof(db, tokens[i].c_str()) << "]\n";
    }
}

void handle_del(KivaDB* db, const std::vector<std::string>& tokens, const char* db_path) {
    if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
        kiva_close(db); remove(db_path);
        // Note: Le ré-ouvrir nécessite de modifier le pointeur dans le main, 
        // ou d'accepter une fermeture propre. Ici, on informe simplement.
        std::cout << "Database file cleared. Please restart shell.\n";
    } else {
        for (size_t i = 1; i < tokens.size(); i++) {
            if (tokens[i] == "and") continue;
            if (kiva_delete(db, tokens[i].c_str()) == KIVA_OK) std::cout << "Deleted: " << tokens[i] << "\n";
            else std::cout << "Not found: " << tokens[i] << "\n";
        }
    }
}