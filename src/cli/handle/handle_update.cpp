#include "../commands.hpp"
#include <iostream>
#include <cstring>

void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        // Ignorer le mot-clé de liaison "and"[cite: 2]
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        // On avance l'index si un type est explicitement spécifié (ex: update string key "val")
        // La variable 'forced' est retirée ici pour supprimer le warning de compilation[cite: 3]
        if (tokens[i] == "string" || tokens[i] == "number" || tokens[i] == "boolean") { 
            i++; 
        }

        // Vérification de sécurité pour ne pas déborder du vecteur de tokens
        if (i + 1 >= tokens.size()) break;

        // Validation du mot-clé réservé : interdiction d'utiliser 'set', 'get', etc. comme clé[cite: 3]
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is reserved.\n";
            i += 2; 
            continue;
        }

        // Vérification de l'existence : update ne fonctionne que si la clé existe déjà[cite: 2]
        const char* current_type_str = kiva_typeof(*db, tokens[i].c_str());
        if (strcmp(current_type_str, "none") == 0) {
            std::cout << "Error: Key '" << tokens[i] << "' not found.\n";
            i += 2; 
            continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];
        KivaType detected_type = KIVA_TYPE_UNKNOWN;

        // Analyse du format de la nouvelle valeur (Guillemets obligatoires pour les strings)[cite: 2]
        if (is_string_quote(val_delim)) {
            detected_type = KIVA_TYPE_STRING;
        } else {
            detected_type = kiva_identify_type(val_str.c_str());
            if (detected_type == KIVA_TYPE_STRING) {
                std::cout << "Error: String values must be quoted.\n";
                i += 2; 
                continue;
            }
        }

        // Comparaison avec le type existant : interdiction de changer le type lors d'un update[cite: 2]
        std::string actual(current_type_str);
        if ((actual == "string" && detected_type != KIVA_TYPE_STRING) ||
            (actual == "number" && detected_type != KIVA_TYPE_NUMBER) ||
            (actual == "boolean" && detected_type != KIVA_TYPE_BOOLEAN)) {
            std::cout << "Error: Type mismatch. Cannot update [" << actual << "] with " 
                      << (is_string_quote(val_delim) ? "quoted string" : "bare value") << ".\n";
            i += 2; 
            continue;
        }

        // Tout est validé : Mise à jour de la valeur
        // Le TTL est mis à 0 pour ne pas modifier l'expiration actuelle lors d'un simple update[cite: 2]
        kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), detected_type, 0);
        std::cout << "OK: " << tokens[i] << " updated.\n";
        
        i += 2; // Passer à la paire suivante
    }
}