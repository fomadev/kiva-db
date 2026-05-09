/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère la commande HAS.
 * Formats supportés :
 * - has <key>          : Vérifie l'existence simple.
 * - has <type> <key>   : Vérifie l'existence et la correspondance du type.
 */
void handle_has(KivaDB** db, const std::vector<std::string>& tokens) {
    if (!db || !*db || tokens.size() < 2) {
        return; 
    }

    size_t n = tokens.size();
    const char* key = nullptr;
    const char* expected_type = nullptr;

    // 1. Détermination du format selon le nombre de tokens
    if (n == 2) {
        // Format: has <key>
        key = tokens[1].c_str();
    } else if (n >= 3) {
        // Format: has <type> <key>
        expected_type = tokens[1].c_str();
        key = tokens[2].c_str();
    }

    // 2. Utilisation de kiva_typeof (performant car consulte uniquement l'index en RAM)
    const char* actual_type = kiva_typeof(*db, key);

    // 3. Logique de réponse
    if (strcmp(actual_type, "none") != 0 && strcmp(actual_type, "undefined") != 0) {
        if (expected_type) {
            // Cas où l'utilisateur demande une validation de type spécifique
            if (strcmp(actual_type, expected_type) == 0) {
                std::cout << "Yes: Key '" << key << "' exists with type [" << actual_type << "].\n";
            } else {
                std::cout << "No: Key '" << key << "' exists but type is [" << actual_type 
                          << "] (expected [" << expected_type << "]).\n";
            }
        } else {
            // Cas de vérification d'existence simple
            std::cout << "Yes: Key '" << key << "' exists.\n";
        }
    } else {
        // La clé n'existe pas
        std::cout << "No: Key '" << key << "' does not exist.\n";
    }
}