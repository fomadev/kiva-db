#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère la commande GET avec support du typage explicite et gestion multi-clés.
 * Le paramètre delimiters est commenté pour éviter le warning 'unused parameter'.
 */
void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& /* delimiters */) {
    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    for (size_t i = 1; i < tokens.size(); i++) {
        // Détection du type forcé pour la lecture
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
        
        // Ignorer le mot-clé de liaison
        if (tokens[i] == "and") {
            continue; 
        }

        // Vérification du type enregistré en base avant la lecture
        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());
        
        if (requested_type != KIVA_TYPE_UNKNOWN) {
            std::string actual(actual_type_str);
            bool mismatch = false;
            
            // Validation de la correspondance entre le type demandé et le type réel[cite: 2]
            if (requested_type == KIVA_TYPE_STRING && actual != "string") mismatch = true;
            if (requested_type == KIVA_TYPE_NUMBER && actual != "number") mismatch = true;
            if (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean") mismatch = true;

            if (mismatch) {
                std::cout << "Error: Type mismatch. '" << tokens[i] << "' is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN; // Reset pour la clé suivante
                continue;
            }
        }

        // Récupération et affichage de la valeur[cite: 2]
        char* val = kiva_get(*db, tokens[i].c_str());
        if (val) {
            std::cout << tokens[i] << " -> " << val << " [" << actual_type_str << "]\n";
            free(val); // Libération de la mémoire allouée par le moteur C[cite: 2]
        } else {
            std::cout << tokens[i] << " -> Not found\n";
        }
        
        // Reset crucial pour permettre une requête différente après un "and"[cite: 2]
        requested_type = KIVA_TYPE_UNKNOWN; 
    }
}