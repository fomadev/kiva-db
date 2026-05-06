#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère la commande GET avec support du typage explicite et gestion multi-clés.
 * Permet de récupérer des valeurs tout en validant leur type si demandé.
 */
void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    for (size_t i = 1; i < tokens.size(); i++) {
        // 1. Détection du type forcé pour la lecture (ex: get number age)
        if (tokens[i] == "string") { 
            requested_type = KIVA_TYPE_STRING; 
            continue; 
        }
        if (tokens[i] == "number") { 
            requested_type = KIVA_TYPE_NUMBER; 
            continue; 
        }
        if (tokens[i] == "boolean") { 
            requested_type = KIVA_TYPE_BOOLEAN; 
            continue; 
        }
        
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") {
            continue; 
        }

        // 2. PROTECTION : Une clé ne doit jamais être entre guillemets lors d'un GET
        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted. Keys must be bare text.\n";
            requested_type = KIVA_TYPE_UNKNOWN; // Reset pour la clé suivante
            continue;
        }

        // 3. Récupération du type réel en base
        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());
        
        // 4. Validation de la concordance si un type a été spécifié
        if (requested_type != KIVA_TYPE_UNKNOWN) {
            // Si la clé n'existe pas
            if (strcmp(actual_type_str, "undefined") == 0 || strcmp(actual_type_str, "none") == 0) {
                std::cout << tokens[i] << ": (nil)\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }

            std::string actual(actual_type_str);
            bool mismatch = (requested_type == KIVA_TYPE_STRING && actual != "string") ||
                            (requested_type == KIVA_TYPE_NUMBER && actual != "number") ||
                            (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean");

            if (mismatch) {
                std::cout << "Error: Type mismatch. '" << tokens[i] << "' is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN; // Reset pour la clé suivante
                continue;
            }
        }

        // 5. Récupération de la valeur
        char* res = kiva_get(*db, tokens[i].c_str());
        
        if (res) {
            // Affichage formaté avec le type pour la clarté du shell
            std::cout << tokens[i] << ": " << res << " [" << actual_type_str << "]\n";
            free(res); // Crucial : libérer la mémoire allouée par le moteur C[cite: 2]
        } else {
            std::cout << tokens[i] << ": (nil)\n";
        }

        // 6. Reset du type pour permettre une requête différente après un "and"[cite: 2]
        requested_type = KIVA_TYPE_UNKNOWN;
    }
}