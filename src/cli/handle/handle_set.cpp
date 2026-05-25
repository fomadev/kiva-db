/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <vector>
#include <string>

// Déclarations des fonctions utilitaires externes
bool is_reserved_keyword(const std::string& key);
bool is_string_quote(char d);
bool is_bare(char d);
bool is_backtick(char d);

/**
 * Gère la commande SET avec support du TTL, du typage forcé et du chaînage 'and'.
 * Alignement immédiat de l'index sur la clé pour garantir la synchronisation avec le parser.
 */
void handle_set(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
    
    // 1. Pré-scan pour extraire le TTL global s'il existe
    for (size_t j = 0; j < tokens.size(); j++) {
        if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
            try { 
                global_ttl = std::stoi(tokens[j+1]); 
            } catch(...) { 
                global_ttl = 0; 
            }
        }
    }

    // 2. Traitement des paires Clé/Valeur
    for (size_t i = 1; i < tokens.size(); ) {
        
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }
        
        // Ignorer le bloc "ttl <val>" qui a déjà été traité au pré-scan
        if (tokens[i] == "ttl") {
            i += 2; 
            continue;
        }

        KivaType forced = KIVA_TYPE_UNKNOWN;

        // Si le token actuel est un modificateur de type, on l'enregistre et on avance i.
        // Après ce bloc, 'i' pointe STRUCTURELLEMENT et TOUJOURS sur la clé physique.
        if (tokens[i] == "string")       { forced = KIVA_TYPE_STRING;  i++; }
        else if (tokens[i] == "number")  { forced = KIVA_TYPE_NUMBER;  i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        // Sécurité : Vérifie qu'il reste bien une clé et une valeur à consommer
        if (i >= tokens.size() || i + 1 >= tokens.size()) {
            std::cout << "Error: Syntax error. Missing key or value.\n";
            break;
        }

        // L'index 'i' étant parfaitement aligné, la correspondance est absolue
        char key_delim = delimiters[i];
        char val_delim = delimiters[i+1];
        std::string key_str = tokens[i];
        std::string val_str = tokens[i+1];

        // Calcul de la position de la paire suivante (i + Clé + Valeur)
        size_t next_i = i + 2;

        // --- VALIDATION DE LA CLÉ ---
        if (is_reserved_keyword(key_str)) {
            std::cout << "Error: '" << key_str << "' is a reserved keyword and cannot be used as a key.\n";
            i = next_i; continue;
        }

        if (is_string_quote(key_delim)) {
            std::cout << "Error: Key '" << key_str << "' cannot use quotes. Use bare text or backticks (``).\n";
            i = next_i; continue;
        }

        // --- VALIDATION DU TYPE ET DU FORMAT DE LA VALEUR ---
        if (forced == KIVA_TYPE_UNKNOWN) {
            if (is_string_quote(val_delim)) {
                forced = KIVA_TYPE_STRING;
            } 
            else {
                KivaType inferred = kiva_identify_type(val_str.c_str()); 
                if (inferred == KIVA_TYPE_STRING) {
                    std::cout << "Error: String values like '" << val_str << "' must be quoted (\"\" or '').\n";
                    i = next_i; continue;
                }
                forced = inferred;
            }
        } 
        else {
            if (forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN) {
                if (!is_bare(val_delim)) {
                    std::cout << "Error: Numbers and Booleans must not be quoted.\n";
                    i = next_i; continue;
                }
            } 
            else if (forced == KIVA_TYPE_STRING) {
                if (!is_string_quote(val_delim)) {
                    std::cout << "Error: Explicit 'string' type requires quotes \"\" or ''.\n";
                    i = next_i; continue;
                }
            }
        }

        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << key_str << "' cannot use backticks.\n";
            i = next_i; continue;
        }

        // --- VÉRIFICATION D'EXISTENCE ---
        char* exists = kiva_get(*db, key_str.c_str());
        if (exists) {
            std::cout << "Error: Key '" << key_str << "' already exists. Use 'update' to change it.\n";
            free(exists); 
            i = next_i; continue;
        }

        // --- PERSISTENCE ---
        KivaStatus status = kiva_set_ex(*db, key_str.c_str(), val_str.c_str(), forced, global_ttl);
        if (status == KIVA_OK) {
            std::cout << "OK: " << key_str << " saved.\n";
        } else {
            std::cout << "Error: Could not save '" << key_str << "' (Internal error).\n";
        }
        
        i = next_i; 
    }
}