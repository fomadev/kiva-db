#include "../commands.hpp"
#include <iostream>
#include <cstring>

void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        if (i + 1 >= tokens.size()) break;

        // Validation du mot-clé réservé
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is reserved.\n";
            i += 2; continue;
        }

        // Vérification de l'existence
        const char* current_type_str = kiva_typeof(*db, tokens[i].c_str());
        if (strcmp(current_type_str, "none") == 0) {
            std::cout << "Error: Key '" << tokens[i] << "' not found.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];
        KivaType detected_type = KIVA_TYPE_UNKNOWN;

        // Analyse du format de la nouvelle valeur
        if (is_string_quote(val_delim)) {
            detected_type = KIVA_TYPE_STRING;
        } else {
            detected_type = kiva_identify_type(val_str.c_str());
            if (detected_type == KIVA_TYPE_STRING) {
                std::cout << "Error: String values must be quoted.\n";
                i += 2; continue;
            }
        }

        // Comparaison avec le type existant en base
        std::string actual(current_type_str);
        if ((actual == "string" && detected_type != KIVA_TYPE_STRING) ||
            (actual == "number" && detected_type != KIVA_TYPE_NUMBER) ||
            (actual == "boolean" && detected_type != KIVA_TYPE_BOOLEAN)) {
            std::cout << "Error: Type mismatch. Cannot update [" << actual << "] with " 
                      << (is_string_quote(val_delim) ? "quoted string" : "bare value") << ".\n";
            i += 2; continue;
        }

        // Tout est OK : Mise à jour sans changer le TTL actuel (0 ici signifie pas de changement)
        kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), detected_type, 0);
        std::cout << "OK: " << tokens[i] << " updated.\n";
        i += 2;
    }
}