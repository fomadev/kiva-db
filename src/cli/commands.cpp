#include "commands.hpp"
#include <iostream>
#include <cstdlib>
#include <vector>
#include <string>

/**
 * Gère la commande SET avec support du TTL et validation des types.
 */
void handle_set(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
    
    // 1. Extraction du TTL global si présent
    for (size_t j = 0; j < tokens.size(); j++) {
        if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
            try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
        }
    }

    // 2. Traitement des paires Clé/Valeur
    for (size_t i = 1; i < tokens.size(); ) {
        // Ignorer les mots-clés de liaison
        if (tokens[i] == "and" || tokens[i] == "ttl") { 
            i += (tokens[i] == "ttl" ? 2 : 1); 
            continue; 
        }
        
        // Détection du type forcé (facultatif)
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        // Vérification de la présence d'une valeur après la clé
        if (i + 1 >= tokens.size()) {
            std::cout << "Error: Key '" << tokens[i] << "' is missing a value.\n";
            break;
        }

        // --- VALIDATION CLI ---

        // Pas de guillemets sur les clés
        if (delimiters[i] != 0) {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes.\n";
            i += 2; continue;
        }

        // Guillemets obligatoires UNIQUEMENT pour les strings
        if (forced == KIVA_TYPE_STRING && delimiters[i+1] == 0) {
            std::cout << "Error: String value for '" << tokens[i] << "' must be quoted.\n";
            i += 2; continue;
        }

        // Vérification de valeur vide
        if (tokens[i+1].empty()) {
            std::cout << "Error: Value for '" << tokens[i] << "' cannot be empty.\n";
            i += 2; continue;
        }

        // Vérification si la clé existe déjà (SET ne doit pas écraser, utiliser UPDATE)
        char* exists = kiva_get(*db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' exists. Use 'update'.\n";
            free(exists); i += 2; continue;
        }

        // 3. Appel au moteur Core
        KivaStatus status = kiva_set_ex(*db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);

        if (status == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        } else if (status == KIVA_ERR_TYPE_MISMATCH) {
            std::cout << "Error: Type mismatch for '" << tokens[i] << "'.\n";
        } else {
            std::cout << "Error: Could not save '" << tokens[i] << "'.\n";
        }
        
        i += 2;
    }
}

/**
 * Gère la récupération de données.
 */
void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        
        if (delimiters[i] != 0) {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted. Use bare text.\n";
            continue;
        }

        char* res = kiva_get(*db, tokens[i].c_str());
        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
        if (res) free(res);
    }
}

/**
 * Gère la mise à jour de clés existantes.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        char* check = kiva_get(*db, tokens[i].c_str());
        if (!check) {
            std::cout << "Error: " << tokens[i] << " not found.\n";
        } else {
            kiva_set(*db, tokens[i].c_str(), tokens[i+1].c_str());
            std::cout << "OK: " << tokens[i] << " updated.\n";
            free(check);
        }
        i += 2;
    }
}

/**
 * Gère le renommage de clés.
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 2 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        if (tokens[i+1] == "to") {
            char* val = kiva_get(*db, tokens[i].c_str());
            if (!val) { 
                std::cout << "Error: Source '" << tokens[i] << "' not found.\n"; 
                i += 3; continue; 
            }

            char* target = kiva_get(*db, tokens[i+2].c_str());
            if (target) { 
                std::cout << "Error: Target '" << tokens[i+2] << "' exists.\n"; 
                free(val); free(target); i += 3; continue; 
            }

            kiva_set(*db, tokens[i+2].c_str(), val); 
            kiva_delete(*db, tokens[i].c_str());
            std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
            
            free(val); i += 3;
        } else {
            i++;
        }
    }
}

/**
 * Affiche le type d'une clé.
 */
void handle_typeof(KivaDB** db, const std::vector<std::string>& tokens) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        std::cout << " -> " << tokens[i] << " is a [" << kiva_typeof(*db, tokens[i].c_str()) << "]\n";
    }
}

/**
 * Gère la suppression (individuelle ou totale).
 */
void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path) {
    // Cas du "del all keys"
    if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
        kiva_close(*db); 
        
        // Suppression physique du fichier
        if (remove(db_path) == 0) {
            // Ré-ouverture immédiate d'une base vide
            *db = kiva_open(db_path); 
            if (*db) {
                std::cout << "Database cleared and re-initialized successfully.\n";
            } else {
                std::cout << "Fatal: Could not re-open database. Check permissions.\n";
            }
        } else {
            std::cout << "Error: Could not delete database file.\n";
        }
    } 
    // Cas de suppressions individuelles
    else {
        for (size_t i = 1; i < tokens.size(); i++) {
            if (tokens[i] == "and") continue;
            if (kiva_delete(*db, tokens[i].c_str()) == KIVA_OK) {
                std::cout << "Deleted: " << tokens[i] << "\n";
            } else {
                std::cout << "Not found: " << tokens[i] << "\n";
            }
        }
    }
}