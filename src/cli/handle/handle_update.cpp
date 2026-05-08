/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère la mise à jour des clés existantes.
 * Vérifie : Existence, mots-clés réservés, intégrité des types et formatage des chaînes.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        // Détection du type forcé par l'utilisateur
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        // Vérification de sécurité pour ne pas déborder
        if (i + 1 >= tokens.size()) break;

        // 1. Protection contre les mots-clés réservés (set, get, etc.)
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is reserved.\n";
            i += 2; 
            continue;
        }

        // 2. Vérification de l'existence : update requiert une clé déjà présente
        const char* current_type_str = kiva_typeof(*db, tokens[i].c_str());
        if (strcmp(current_type_str, "none") == 0) {
            std::cout << "Error: Key '" << tokens[i] << "' not found.\n";
            i += 2; 
            continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];
        KivaType detected_type = KIVA_TYPE_UNKNOWN;

        // 3. Analyse du format de la nouvelle valeur (Quotes obligatoires pour les strings)
        if (is_string_quote(val_delim)) {
            detected_type = KIVA_TYPE_STRING;
        } else {
            detected_type = kiva_identify_type(val_str.c_str());
            // Si c'est détecté comme string mais sans quotes, on refuse
            if (detected_type == KIVA_TYPE_STRING) {
                std::cout << "Error: String values must be quoted (\"\" or '').\n";
                i += 2; 
                continue;
            }
        }

        // 4. Validation de la concordance (Si un type est forcé dans la commande)
        std::string actual(current_type_str);
        if (forced != KIVA_TYPE_UNKNOWN) {
            if ((forced == KIVA_TYPE_STRING && actual != "string") ||
                (forced == KIVA_TYPE_NUMBER && actual != "number") ||
                (forced == KIVA_TYPE_BOOLEAN && actual != "boolean")) {
                std::cout << "Error: Type mismatch. '" << tokens[i] << "' is a [" << actual << "].\n";
                i += 2; 
                continue;
            }
        }

        // 5. Validation du format par rapport au type existant en base
        // Empêche par exemple d'updater un "number" avec une "string"
        if ((actual == "string" && detected_type != KIVA_TYPE_STRING) ||
            (actual == "number" && detected_type != KIVA_TYPE_NUMBER) ||
            (actual == "boolean" && detected_type != KIVA_TYPE_BOOLEAN)) {
            std::cout << "Error: Cannot update [" << actual << "] with a value formatted as [" 
                      << (detected_type == KIVA_TYPE_NUMBER ? "number" : (detected_type == KIVA_TYPE_BOOLEAN ? "boolean" : "string")) 
                      << "].\n";
            i += 2; 
            continue;
        }

        // 6. Exécution de la mise à jour
        // On utilise detected_type pour l'écriture. Le TTL est à 0 pour ne pas changer l'existant.
        kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), detected_type, 0);
        std::cout << "OK: " << tokens[i] << " updated.\n";
        
        i += 2; // Passage à la paire suivante
    }
}