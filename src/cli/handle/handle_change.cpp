/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * handle_change (v2.1.3 - Refactoring Mode)
 * Supporte :
 * - change <old> to <new>                  (Simple rename)
 * - change <old> to <new> <val>            (Rename + New Value)
 * - change <old> to <type> <new> <val>     (Rename + Type Change + New Value)
 * - change <type> <old> to <type> <new>    (Explicit rename)
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens) {
    size_t n = tokens.size();
    if (n < 3) {
        std::cerr << "Error: Invalid change syntax.\nUsage: change [type] <old> to [type] <new> [value]" << std::endl;
        return;
    }

    // 1. Localisation de l'ancienne clé (old_key)
    std::string old_key;
    size_t to_index = 0;

    // Si le premier argument est un type (ex: change string u to ...)
    if (is_kiva_type(tokens[1])) {
        if (n < 4) { std::cerr << "Error: Missing source key." << std::endl; return; }
        old_key = tokens[2];
        to_index = 3;
    } else {
        old_key = tokens[1];
        to_index = 2;
    }

    // Validation du mot-clé "to"
    if (to_index >= n || tokens[to_index] != "to") {
        std::cerr << "Error: Missing 'to' keyword in change command." << std::endl;
        return;
    }

    // 2. Localisation de la nouvelle clé (new_key) et du type forcé
    std::string new_key;
    KivaType k_type = KIVA_TYPE_AUTO; 
    size_t val_index = 0;
    size_t after_to = to_index + 1;

    if (after_to < n && is_kiva_type(tokens[after_to])) {
        // Détection du type forcé (ex: to number age 12)
        std::string forced_type = tokens[after_to];
        if (forced_type == "string") k_type = KIVA_TYPE_STRING;
        else if (forced_type == "number") k_type = KIVA_TYPE_NUMBER;
        else if (forced_type == "boolean") k_type = KIVA_TYPE_BOOLEAN;

        if (after_to + 1 >= n) { std::cerr << "Error: Missing target key." << std::endl; return; }
        new_key = tokens[after_to + 1];
        val_index = after_to + 2;
    } else if (after_to < n) {
        // Format standard (ex: to age 12)
        new_key = tokens[after_to];
        val_index = after_to + 1;
    } else {
        std::cerr << "Error: Missing target key." << std::endl;
        return;
    }

    // 3. Vérification de l'existence de la source
    const char* actual_type_str = kiva_typeof(*db, old_key.c_str());
    if (std::strcmp(actual_type_str, "none") == 0 || std::strcmp(actual_type_str, "undefined") == 0) {
        std::cerr << "Error: Source key '" << old_key << "' not found." << std::endl;
        return;
    }

    // 4. Logique d'exécution
    
    // CAS A : On garde la valeur actuelle (Simple Rename)
    if (val_index >= n) {
        KivaStatus status = kiva_rename(*db, old_key.c_str(), new_key.c_str());
        if (status == KIVA_OK) {
            std::cout << "Renamed: " << old_key << " -> " << new_key << " (Value preserved)" << std::endl;
        } else {
            std::cerr << "Error: Failed to rename. Check if target already exists." << std::endl;
        }
    } 
    // CAS B : Migration avec nouvelle valeur (Atomic Move)
    else {
        std::string new_value = tokens[val_index];
        
        // 1. Récupérer le type original si aucun type n'est forcé
        if (k_type == KIVA_TYPE_AUTO) {
            if (std::strcmp(actual_type_str, "string") == 0) k_type = KIVA_TYPE_STRING;
            else if (std::strcmp(actual_type_str, "number") == 0) k_type = KIVA_TYPE_NUMBER;
            else if (std::strcmp(actual_type_str, "boolean") == 0) k_type = KIVA_TYPE_BOOLEAN;
        }

        // 2. Supprimer l'ancienne clé
        kiva_delete(*db, old_key.c_str());
        
        // 3. Créer la nouvelle clé avec la nouvelle valeur
        // On utilise kiva_set_ex avec TTL=0 (persistant)
        KivaStatus status = kiva_set_ex(*db, new_key.c_str(), new_value.c_str(), k_type, 0);
        
        if (status == KIVA_OK) {
            std::cout << "Migrated: " << old_key << " -> " << new_key << " (New value: " << new_value << ")" << std::endl;
        } else {
            std::cerr << "Error: Failed to create target key with new value." << std::endl;
        }
    }
}