#include "commands.hpp"
#include <iostream>
#include <cstdlib>

/**
 * SET: Ajoute des clés avec support optionnel du TTL et typage forcé.
 * Exemple: set number age "25" and string name "Kiva" ttl 3600
 */
void handle_set(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
    // Recherche du TTL global dans la commande
    for (size_t j = 0; j < tokens.size(); j++) {
        if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
            try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
        }
    }

    for (size_t i = 1; i < tokens.size(); ) {
        // Ignorer les mots de liaison et le paramètre ttl déjà traité
        if (tokens[i] == "and" || tokens[i] == "ttl") { 
            i += (tokens[i] == "ttl" ? 2 : 1); continue; 
        }
        
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        if (i + 1 >= tokens.size()) break;

        // Validation syntaxique des guillemets
        if (delimiters[i] == '"' || delimiters[i] == '\'') {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes.\n";
            i += 2; continue;
        }
        if (delimiters[i+1] != '"' && delimiters[i+1] != '\'') {
            std::cout << "Error: Value for '" << tokens[i] << "' must be quoted.\n";
            i += 2; continue;
        }

        // Vérification d'existence (SET ne doit pas écraser, utiliser UPDATE)
        char* exists = kiva_get(db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' exists. Use 'update'.\n";
            free(exists); i += 2; continue;
        }

        // Appel au Core avec validation de type
        KivaStatus status = kiva_set_ex(db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);
        
        if (status == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        } else if (status == KIVA_ERR_TYPE_MISMATCH) {
            std::cout << "Error: Value does not match the forced type [" << tokens[i-1] << "].\n";
        } else {
            std::cout << "Error: Could not save " << tokens[i] << ".\n";
        }
        i += 2;
    }
}

/**
 * GET: Récupère et affiche les valeurs.
 */
void handle_get(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        if (delimiters[i] == '"' || delimiters[i] == '\'') {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted. Use bare text.\n";
            continue;
        }
        char* res = kiva_get(db, tokens[i].c_str());
        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
        if (res) free(res);
    }
}

/**
 * UPDATE: Modifie une valeur existante.
 */
void handle_update(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }
        
        if (delimiters[i] == '"' || delimiters[i] == '\'' || (delimiters[i+1] != '"' && delimiters[i+1] != '\'')) {
            std::cout << "Error: Syntax error in update. Keys must be bare, values quoted.\n"; i += 2; continue;
        }

        char* check = kiva_get(db, tokens[i].c_str());
        if (!check) {
            std::cout << "Error: " << tokens[i] << " not found.\n";
        } else {
            kiva_set(db, tokens[i].c_str(), tokens[i+1].c_str());
            std::cout << "OK: " << tokens[i] << " updated.\n";
            free(check);
        }
        i += 2;
    }
}

/**
 * CHANGE: Renomme une clé.
 */
void handle_change(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 2 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        if (tokens[i+1] == "to") {
            if (delimiters[i] != 0 || delimiters[i+2] != 0) {
                std::cout << "Error: Keys cannot be quoted for rename.\n";
                i += 3; continue;
            }

            char* val = kiva_get(db, tokens[i].c_str());
            if (!val) { 
                std::cout << "Error: Source '" << tokens[i] << "' not found.\n"; 
                i += 3; continue; 
            }

            char* target = kiva_get(db, tokens[i+2].c_str());
            if (target) { 
                std::cout << "Error: Target '" << tokens[i+2] << "' already exists.\n"; 
                free(val); free(target); i += 3; continue; 
            }

            kiva_set(db, tokens[i+2].c_str(), val); 
            kiva_delete(db, tokens[i].c_str());
            std::cout << "Renamed: " << tokens[i] << " -> " << tokens[i+2] << "\n";
            
            free(val); i += 3;
        } else { i++; }
    }
}

/**
 * TYPEOF: Affiche le type stocké.
 */
void handle_typeof(KivaDB* db, const std::vector<std::string>& tokens) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        std::cout << " -> " << tokens[i] << " is a [" << kiva_typeof(db, tokens[i].c_str()) << "]\n";
    }
}

/**
 * DEL: Supprime une clé ou réinitialise la base.
 * Note: Utilise KivaDB** pour pouvoir ré-assigner le pointeur après un "del all keys".
 */
void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path) {
    if (tokens.size() == 3 && tokens[1] == "all" && tokens[2] == "keys") {
        // Fermeture propre
        kiva_close(*db); 
        
        // Suppression physique
        if (remove(db_path) == 0) {
            // Réouverture immédiate : recrée un fichier propre avec Header V2
            *db = kiva_open(db_path); 
            if (*db) {
                std::cout << "Database cleared and re-initialized successfully.\n";
            } else {
                std::cout << "Fatal: Could not re-open database. Please restart.\n";
            }
        } else {
            std::cout << "Error: Could not delete database file.\n";
            *db = kiva_open(db_path); // Tenter de réouvrir l'existant au moins
        }
    } else {
        // Suppression individuelle
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