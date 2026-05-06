#include "../commands.hpp"
#include <iostream>

// On déclare les fonctions d'utils pour qu'elles soient visibles ici
bool is_reserved_keyword(const std::string& key);
bool is_string_quote(char d);
bool is_bare(char d);
bool is_backtick(char d);

void handle_set(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
    
    for (size_t j = 0; j < tokens.size(); j++) {
        if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
            try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
        }
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

        if (i + 1 >= tokens.size()) {
            std::cout << "Error: Key '" << tokens[i] << "' is missing a value.\n";
            break;
        }

        // Validation Key
        if (is_string_quote(delimiters[i]) || is_reserved_keyword(tokens[i])) {
            std::cout << "Error: Invalid or reserved key '" << tokens[i] << "'.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];

        // Détection du type
        if (forced == KIVA_TYPE_UNKNOWN) {
            if (is_string_quote(val_delim)) forced = KIVA_TYPE_STRING;
            else {
                KivaType inferred = kiva_identify_type(val_str.c_str());
                if (inferred == KIVA_TYPE_STRING) {
                    std::cout << "Error: String '" << val_str << "' must be quoted.\n";
                    i += 2; continue;
                }
                forced = inferred;
            }
        } else {
            // Validation format vs forced type
            if ((forced == KIVA_TYPE_STRING && !is_string_quote(val_delim)) ||
                ((forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN) && !is_bare(val_delim))) {
                std::cout << "Error: Value format mismatch for type.\n";
                i += 2; continue;
            }
        }

        if (kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), forced, global_ttl) == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        }
        i += 2;
    }
}