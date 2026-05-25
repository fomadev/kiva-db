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
 * Gère la mise à jour des clés existantes.
 * Synchronisation absolue des délimiteurs par indexation relative.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i < tokens.size(); ) {
        
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        // --- CALCUL DES INDEX RELATIFS (Zéro décalage) ---
        KivaType forced = KIVA_TYPE_UNKNOWN;
        size_t key_index = i;
        
        if (tokens[i] == "string")       { forced = KIVA_TYPE_STRING;  key_index = i + 1; }
        else if (tokens[i] == "number")  { forced = KIVA_TYPE_NUMBER;  key_index = i + 1; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; key_index = i + 1; }

        size_t val_index = key_index + 1;

        // Sécurité anti-débordement
        if (key_index >= tokens.size() || val_index >= tokens.size()) {
            std::cout << "Error: Syntax error. Missing key or value.\n";
            break;
        }

        // Extraction isolée et synchronisée
        char key_delim = delimiters[key_index];
        char val_delim = delimiters[val_index];
        std::string key_str = tokens[key_index];
        std::string val_str = tokens[val_index];

        // Index d'avancement pour sécuriser les 'continue'
        size_t next_i = val_index + 1;

        // 1. Protection contre les mots-clés réservés
        if (is_reserved_keyword(key_str)) {
            std::cout << "Error: '" << key_str << "' is a reserved keyword.\n";
            i = next_i; continue;
        }

        // Vérification des guillemets sur la clé
        if (is_string_quote(key_delim)) {
            std::cout << "Error: Key '" << key_str << "' cannot use quotes. Use bare text or backticks (``).\n";
            i = next_i; continue;
        }

        // 2. Vérification de l'existence
        const char* current_type_str = kiva_typeof(*db, key_str.c_str());
        if (std::strcmp(current_type_str, "none") == 0) {
            std::cout << "Error: Key '" << key_str << "' not found. Use 'set' to create it.\n";
            i = next_i; continue;
        }

        // Interdiction des backticks sur les valeurs
        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << key_str << "' cannot use backticks.\n";
            i = next_i; continue;
        }

        KivaType detected_type = KIVA_TYPE_UNKNOWN;

        // 3. Analyse du format de la nouvelle valeur
        if (is_string_quote(val_delim)) {
            detected_type = KIVA_TYPE_STRING;
        } else {
            detected_type = kiva_identify_type(val_str.c_str());
            if (detected_type == KIVA_TYPE_STRING) {
                std::cout << "Error: String values like '" << val_str << "' must be quoted (\"\" or '').\n";
                i = next_i; continue;
            }
        }

        // 4. Validation de la concordance si un type est forcé explicitement
        std::string actual(current_type_str);
        if (forced != KIVA_TYPE_UNKNOWN) {
            if ((forced == KIVA_TYPE_STRING && actual != "string") ||
                (forced == KIVA_TYPE_NUMBER && actual != "number") ||
                (forced == KIVA_TYPE_BOOLEAN && actual != "boolean")) {
                std::cout << "Error: Type mismatch. '" << key_str << "' is a [" << actual << "].\n";
                i = next_i; continue;
            }
            
            if (forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN) {
                if (!is_bare(val_delim)) {
                    std::cout << "Error: Numbers and Booleans must not be quoted.\n";
                    i = next_i; continue;
                }
            } else if (forced == KIVA_TYPE_STRING) {
                if (!is_string_quote(val_delim)) {
                    std::cout << "Error: Explicit 'string' type requires quotes \"\" or ''.\n";
                    i = next_i; continue;
                }
            }
        }

        // 5. Validation du format par rapport au type en base
        if ((actual == "string" && detected_type != KIVA_TYPE_STRING) ||
            (actual == "number" && detected_type != KIVA_TYPE_NUMBER) ||
            (actual == "boolean" && detected_type != KIVA_TYPE_BOOLEAN)) {
            std::cout << "Error: Cannot update [" << actual << "] with a value formatted as [" 
                      << (detected_type == KIVA_TYPE_NUMBER ? "number" : (detected_type == KIVA_TYPE_BOOLEAN ? "boolean" : "string")) 
                      << "].\n";
            i = next_i; continue;
        }

        // 6. Exécution de la mise à jour
        KivaStatus status = kiva_set_ex(*db, key_str.c_str(), val_str.c_str(), detected_type, 0);
        if (status == KIVA_OK) {
            std::cout << "OK: " << key_str << " updated.\n";
        } else {
            std::cout << "Error: Could not update '" << key_str << "' (Internal error).\n";
        }
        
        // Avancement dynamique précis
        i = next_i; 
    }
}