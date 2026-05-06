#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère la suppression simple, multiple ou totale des clés.
 * Supporte la vérification de type optionnelle : del string <key>
 */
void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path) {
    // Cas spécial : Suppression totale de la base
    if (tokens.size() >= 3 && tokens[1] == "all" && tokens[2] == "keys") {
        kiva_close(*db); 
        if (remove(db_path) == 0) {
            *db = kiva_open(db_path); 
            std::cout << "Database cleared successfully.\n";
        } else {
            std::cout << "Error: Could not delete database file.\n";
        }
        return;
    } 

    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    for (size_t i = 1; i < tokens.size(); i++) {
        // Détection du type de sécurité
        if (tokens[i] == "string") { requested_type = KIVA_TYPE_STRING; continue; }
        if (tokens[i] == "number") { requested_type = KIVA_TYPE_NUMBER; continue; }
        if (tokens[i] == "boolean") { requested_type = KIVA_TYPE_BOOLEAN; continue; }
        if (tokens[i] == "and") continue;

        // Récupération du type actuel pour validation
        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());

        if (requested_type != KIVA_TYPE_UNKNOWN) {
            std::string actual(actual_type_str);
            
            // Si la clé n'existe pas, inutile d'aller plus loin
            if (actual == "none") {
                std::cout << "Not found: " << tokens[i] << "\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }

            bool mismatch = false;
            if (requested_type == KIVA_TYPE_STRING && actual != "string") mismatch = true;
            if (requested_type == KIVA_TYPE_NUMBER && actual != "number") mismatch = true;
            if (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean") mismatch = true;

            if (mismatch) {
                std::cout << "Error: Type mismatch. Cannot delete '" << tokens[i] 
                          << "' because it is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }
        }

        // Suppression effective via l'API C
        if (kiva_delete(*db, tokens[i].c_str()) == KIVA_OK) {
            std::cout << "Deleted: " << tokens[i] << "\n";
        } else {
            std::cout << "Not found: " << tokens[i] << "\n";
        }
        
        requested_type = KIVA_TYPE_UNKNOWN; // Reset pour l'itération suivante
    }
}