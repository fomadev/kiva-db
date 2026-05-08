/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>
#include <vector>
#include <string>

/**
 * Gère la suppression simple, multiple ou totale des clés.
 * Supporte la vérification de type optionnelle (ex: del string <key>)
 * et le nettoyage complet (del all keys).
 */
void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path) {
    // 1. Cas spécial : Suppression totale de la base (Reset complet)
    if (tokens.size() >= 3 && tokens[1] == "all" && tokens[2] == "keys") {
        kiva_close(*db); 
        // Suppression physique du fichier de la base de données
        if (remove(db_path) == 0) {
            *db = kiva_open(db_path); // Réouverture d'une base vide
            std::cout << "Database cleared successfully.\n";
        } else {
            std::cout << "Error: Could not delete database file at " << db_path << ".\n";
            // Tentative de réouverture même en cas d'échec de suppression
            *db = kiva_open(db_path);
        }
        return;
    } 

    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    // 2. Boucle de traitement pour les suppressions individuelles ou chaînées
    for (size_t i = 1; i < tokens.size(); i++) {
        // Détection d'un type de sécurité (ex: del number ma_cle)
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

        // 3. Récupération du type actuel pour validation de sécurité
        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());

        if (requested_type != KIVA_TYPE_UNKNOWN) {
            std::string actual(actual_type_str);
            
            // Si la clé n'existe pas, on informe l'utilisateur
            if (actual == "none" || actual == "undefined") {
                std::cout << "Not found: " << tokens[i] << "\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }

            // Vérification de la concordance entre le type demandé et le type réel
            bool mismatch = false;
            if (requested_type == KIVA_TYPE_STRING && actual != "string") mismatch = true;
            if (requested_type == KIVA_TYPE_NUMBER && actual != "number") mismatch = true;
            if (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean") mismatch = true;

            if (mismatch) {
                std::cout << "Error: Type mismatch. Cannot delete '" << tokens[i] 
                          << "' because it is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN; // Reset pour la clé suivante
                continue;
            }
        }

        // 4. Suppression effective via l'API C
        if (kiva_delete(*db, tokens[i].c_str()) == KIVA_OK) {
            std::cout << "Deleted: " << tokens[i] << "\n";
        } else {
            std::cout << "Not found: " << tokens[i] << "\n";
        }
        
        // Reset crucial du type demandé pour l'itération suivante (après un 'and')
        requested_type = KIVA_TYPE_UNKNOWN; 
    }
}