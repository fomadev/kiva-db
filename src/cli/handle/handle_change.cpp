/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>
#include <algorithm>
#include <vector>
#include <string>

/**
 * RÈGLE v2.1.5 : Une clé est valide si elle n'est pas purement numérique.
 */
static bool is_valid_key_name(const std::string& key) {
    if (key.empty()) return false;
    return !std::all_of(key.begin(), key.end(), ::isdigit);
}

/**
 * Helper local pour la détection de type KivaDB.
 */
static bool is_kiva_type_local(const std::string& t) {
    return (t == "string" || t == "number" || t == "boolean");
}

/**
 * handle_change (v2.1.5)
 * Gère le renommage et la migration avec validation sélective du nom de clé.
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    if (!db || !*db) return;

    size_t n = tokens.size();
    if (n < 3) {
        std::cerr << "Error: Invalid change syntax.\nUsage: change [type] <old> to [type] <new> [value]" << std::endl;
        return;
    }

    // 1. Localisation et validation de cohérence de l'ancienne clé
    std::string old_key;
    KivaType forced_old_type = KIVA_TYPE_AUTO;
    size_t to_index = 0;

    if (is_kiva_type_local(tokens[1])) {
        if (n < 4) { std::cerr << "Error: Missing source key." << std::endl; return; }
        
        if (tokens[1] == "string") forced_old_type = KIVA_TYPE_STRING;
        else if (tokens[1] == "number") forced_old_type = KIVA_TYPE_NUMBER;
        else if (tokens[1] == "boolean") forced_old_type = KIVA_TYPE_BOOLEAN;
        
        old_key = tokens[2];
        to_index = 3;
    } else {
        old_key = tokens[1];
        to_index = 2;
    }

    // Vérification de l'existence de la source
    const char* actual_type_str = kiva_typeof(*db, old_key.c_str());
    if (std::strcmp(actual_type_str, "none") == 0 || std::strcmp(actual_type_str, "undefined") == 0) {
        std::cerr << "Error: Source key '" << old_key << "' not found." << std::endl;
        return;
    }

    KivaType actual_enum = (std::strcmp(actual_type_str, "number") == 0) ? KIVA_TYPE_NUMBER :
                           (std::strcmp(actual_type_str, "boolean") == 0) ? KIVA_TYPE_BOOLEAN : KIVA_TYPE_STRING;

    if (forced_old_type != KIVA_TYPE_AUTO && forced_old_type != actual_enum) {
        std::cerr << "Error: Type mismatch for source key. '" << old_key 
                  << "' is actually a '" << actual_type_str << "'." << std::endl;
        return;
    }

    if (to_index >= n || tokens[to_index] != "to") {
        std::cerr << "Error: Missing 'to' keyword. Format: change <old> to <new>" << std::endl;
        return;
    }

    // 2. Localisation de la nouvelle clé et du type cible
    std::string new_key;
    KivaType k_type_target = KIVA_TYPE_AUTO; 
    size_t val_index = 0;
    size_t after_to = to_index + 1;

    if (after_to < n && is_kiva_type_local(tokens[after_to])) {
        std::string forced_target = tokens[after_to];
        if (forced_target == "string") k_type_target = KIVA_TYPE_STRING;
        else if (forced_target == "number") k_type_target = KIVA_TYPE_NUMBER;
        else if (forced_target == "boolean") k_type_target = KIVA_TYPE_BOOLEAN;

        if (after_to + 1 >= n) { std::cerr << "Error: Missing target key." << std::endl; return; }
        new_key = tokens[after_to + 1];
        val_index = after_to + 2;
    } else if (after_to < n) {
        new_key = tokens[after_to];
        val_index = after_to + 1;
    } else {
        std::cerr << "Error: Missing target key." << std::endl;
        return;
    }

    // --- CORRECTION CRITIQUE v2.1.5 : Validation du NOM de la nouvelle clé uniquement ---
    if (!is_valid_key_name(new_key)) {
        std::cerr << "Error: InvalidKeyName: '" << new_key << "' cannot be purely numeric." << std::endl;
        return;
    }

    // 3. Logique d'exécution SÉCURISÉE
    
    // CAS A : Renommage Simple (Valeur préservée)
    if (val_index >= n) {
        if (old_key == new_key) {
            std::cout << "Renamed: " << old_key << " -> " << new_key << " (No change needed)" << std::endl;
            return; 
        }

        if (k_type_target != KIVA_TYPE_AUTO && k_type_target != actual_enum) {
            std::cerr << "Error: Cannot change type without providing a new value." << std::endl;
            return;
        }

        if (kiva_rename(*db, old_key.c_str(), new_key.c_str()) == KIVA_OK) {
            std::cout << "Renamed: " << old_key << " -> " << new_key << " (Type preserved)" << std::endl;
        } else {
            std::cerr << "Error: Failed to rename. Target might already exist." << std::endl;
        }
    } 
    // CAS B : Migration avec Nouvelle Valeur (Safe Pre-Flight Mode)
    else {
        std::string new_value = tokens[val_index];
        char delim = (val_index < delimiters.size()) ? delimiters[val_index] : 0;

        // PHASE 1 : VALIDATION DE LA VALEUR
        if (k_type_target == KIVA_TYPE_AUTO) {
            if (new_value == "true" || new_value == "false") {
                k_type_target = KIVA_TYPE_BOOLEAN;
            } 
            else if (!new_value.empty() && std::all_of(new_value.begin(), new_value.end(), ::isdigit)) {
                k_type_target = KIVA_TYPE_NUMBER;
            } 
            else {
                if (delim != '\"' && delim != '\'') {
                    std::cerr << "Error: String values must be enclosed in quotes." << std::endl;
                    return; 
                }
                k_type_target = KIVA_TYPE_STRING;
            }
        }
        else if (k_type_target == KIVA_TYPE_STRING) {
            if (delim != '\"' && delim != '\'') {
                std::cerr << "Error: Explicit string type requires quotes." << std::endl;
                return;
            }
        }

        // PHASE 2 : EXÉCUTION
        if (old_key != new_key) {
            kiva_delete(*db, old_key.c_str());
        }

        KivaStatus status = kiva_set_ex(*db, new_key.c_str(), new_value.c_str(), k_type_target, 0);
        
        if (status == KIVA_OK) {
            std::string t_final = (k_type_target == KIVA_TYPE_NUMBER) ? "number" : 
                                  (k_type_target == KIVA_TYPE_BOOLEAN) ? "boolean" : "string";
            std::cout << "Migrated: " << old_key << " -> " << new_key << " (Type: " << t_final << ")" << std::endl;
        } else {
            std::cerr << "Error: Migration failed during insertion." << std::endl;
        }
    }
}