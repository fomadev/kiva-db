/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>
#include <vector>
#include <string>

// Déclarations des fonctions utilitaires externes
bool is_reserved_keyword(const std::string& key);
bool is_string_quote(char d);
bool is_bare(char d);
bool is_backtick(char d);

/**
 * Gère la mise à jour des clés existantes avec chaînage dynamique.
 * Avancement contrôlé de l'index pour éliminer les décalages d'analyse.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    
    // i++ supprimé pour un contrôle manuel rigoureux de la progression
    for (size_t i = 1; i < tokens.size(); ) {
        
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        // Détection du type forcé par l'utilisateur
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string")       { forced = KIVA_TYPE_STRING;  i++; }
        else if (tokens[i] == "number")  { forced = KIVA_TYPE_NUMBER;  i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        // Sécurité anti-débordement après décalage potentiel du type
        if (i >= tokens.size() || i + 1 >= tokens.size()) {
            std::cout << "Error: Syntax error. Missing key or value.\n";
            break;
        }

        // Isolement synchronisé des variables locales
        char key_delim = delimiters[i];
        char val_delim = delimiters[i+1];
        std::string key_str = tokens[i];
        std::string val_str = tokens[i+1];

        // 1. Protection contre les mots-clés réservés
        if (is_reserved_keyword(key_str)) {
            std::cout << "Error: '" << key_str << "' is a reserved keyword.\n";
            i += 2; continue;
        }

        // Garantir que la clé n'est pas encapsulée par des guillemets
        if (is_string_quote(key_delim)) {
            std::cout << "Error: Key '" << key_str << "' cannot use quotes. Use bare text or backticks (``).\n";
            i += 2; continue;
        }

        // 2. Vérification de l'existence (Spécifique à UPDATE)
        const char* current_type_str = kiva_typeof(*db, key_str.c_str());
        if (std::strcmp(current_type_str, "none") == 0) {
            std::cout << "Error: Key '" << key_str << "' not found. Use 'set' to create it.\n";
            i += 2; continue;
        }

        // Interdiction des backticks sur les valeurs
        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << key_str << "' cannot use backticks.\n";
            i += 2; continue;
        }

        KivaType detected_type = KIVA_TYPE_UNKNOWN;

        // 3. Analyse du format de la nouvelle valeur
        if (is_string_quote(val_delim)) {
            detected_type = KIVA_TYPE_STRING;
        } else {
            detected_type = kiva_identify_type(val_str.c_str());
            if (detected_type == KIVA_TYPE_STRING) {
                std::cout << "Error: String values like '" << val_str << "' must be quoted (\"\" or '').\n";
                i += 2; continue;
            }
        }

        // 4. Validation de la concordance si un type est forcé explicitement
        std::string actual(current_type_str);
        if (forced != KIVA_TYPE_UNKNOWN) {
            if ((forced == KIVA_TYPE_STRING && actual != "string") ||
                (forced == KIVA_TYPE_NUMBER && actual != "number") ||
                (forced == KIVA_TYPE_BOOLEAN && actual != "boolean")) {
                std::cout << "Error: Type mismatch. '" << key_str << "' is a [" << actual << "].\n";
                i += 2; continue;
            }
            
            if (forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN) {
                if (!is_bare(val_delim)) {
                    std::cout << "Error: Numbers and Booleans must not be quoted.\n";
                    i += 2; continue;
                }
            } else if (forced == KIVA_TYPE_STRING) {
                if (!is_string_quote(val_delim)) {
                    std::cout << "Error: Explicit 'string' type requires quotes \"\" or ''.\n";
                    i += 2; continue;
                }
            }
        }

        // 5. Validation du format par rapport au type physique en base
        if ((actual == "string" && detected_type != KIVA_TYPE_STRING) ||
            (actual == "number" && detected_type != KIVA_TYPE_NUMBER) ||
            (actual == "boolean" && detected_type != KIVA_TYPE_BOOLEAN)) {
            std::cout << "Error: Cannot update [" << actual << "] with a value formatted as [" 
                      << (detected_type == KIVA_TYPE_NUMBER ? "number" : (detected_type == KIVA_TYPE_BOOLEAN ? "boolean" : "string")) 
                      << "].\n";
            i += 2; continue;
        }

        // 6. Exécution de la mise à jour
        KivaStatus status = kiva_set_ex(*db, key_str.c_str(), val_str.c_str(), detected_type, 0);
        if (status == KIVA_OK) {
            std::cout << "OK: " << key_str << " updated.\n";
        } else {
            std::cout << "Error: Could not update '" << key_str << "' (Internal error).\n";
        }
        
        // Avancement contrôlé
        i += 2; 
    }
}