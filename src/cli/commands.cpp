#include "commands.hpp"
#include <iostream>
#include <cstdlib>
#include <vector>
#include <string>
#include <cstring>

// --- Fonctions utilitaires internes pour la validation ---

static bool is_string_quote(char d) { return d == '"' || d == '\''; }
static bool is_backtick(char d) { return d == '`'; }
static bool is_bare(char d) { return d == 0; }

/**
 * Gère la commande SET avec validation stricte des types et des délimiteurs.
 * RÈGLE : Si une valeur est entre guillemets, elle est forcée en STRING.
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
        if (tokens[i] == "and" || tokens[i] == "ttl") { 
            i += (tokens[i] == "ttl" ? 2 : 1); 
            continue; 
        }
        
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        if (i + 1 >= tokens.size()) {
            std::cout << "Error: Key '" << tokens[i] << "' is missing a value.\n";
            break;
        }

        // --- VALIDATION DES QUOTES ET DÉTERMINATION DU TYPE ---

        // Clé : Pas de " " ou ' ', seulement nu ou ` `
        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes. Use bare text or ``.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];

        // LOGIQUE DE DÉTECTION AUTOMATIQUE (Si pas de type forcé explicitement)
        if (forced == KIVA_TYPE_UNKNOWN) {
            if (is_string_quote(val_delim)) {
                // RÈGLE : Présence de quotes = Forçage en STRING (ex: "74" -> string)
                forced = KIVA_TYPE_STRING;
            }
            // Si bare (0), KivaDB (côté C) fera l'inférence naturelle (number/boolean)
        } 
        else {
            // VALIDATION SI TYPE FORCÉ EXPLICITEMENT
            if (forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN) {
                if (!is_bare(val_delim)) {
                    std::cout << "Error: Numbers and Booleans must not be quoted.\n";
                    i += 2; continue;
                }
            } 
            else if (forced == KIVA_TYPE_STRING) {
                if (!is_string_quote(val_delim)) {
                    std::cout << "Error: Explicit 'string' type requires quotes \"\" or ''.\n";
                    i += 2; continue;
                }
            }
        }

        // Cas particulier : on refuse les backticks sur les valeurs dans tous les cas
        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << tokens[i] << "' cannot use backticks.\n";
            i += 2; continue;
        }

        // Vérification d'existence pour SET (mode strict)
        char* exists = kiva_get(*db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' exists. Use 'update'.\n";
            free(exists); i += 2; continue;
        }

        KivaStatus status = kiva_set_ex(*db, tokens[i].c_str(), tokens[i+1].c_str(), forced, global_ttl);
        if (status == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        } else {
            std::cout << "Error: Could not save '" << tokens[i] << "'.\n";
        }
        
        i += 2;
    }
}

/**
 * Gère la mise à jour avec protection du type existant.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        if (i + 1 >= tokens.size()) break;

        // Récupérer le type actuel en base
        const char* current_type = kiva_typeof(*db, tokens[i].c_str());
        
        if (strcmp(current_type, "none") == 0) {
            std::cout << "Error: Key '" << tokens[i] << "' not found.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];

        // Détection automatique pour la mise à jour si pas de type forcé
        if (forced == KIVA_TYPE_UNKNOWN && is_string_quote(val_delim)) {
            forced = KIVA_TYPE_STRING;
        }

        // Protection : si on précise un type, il doit correspondre à l'existant
        if (forced != KIVA_TYPE_UNKNOWN) {
            std::string type_str = current_type;
            bool mismatch = false;
            if (forced == KIVA_TYPE_STRING && type_str != "string") mismatch = true;
            if (forced == KIVA_TYPE_NUMBER && type_str != "number") mismatch = true;
            if (forced == KIVA_TYPE_BOOLEAN && type_str != "boolean") mismatch = true;

            if (mismatch) {
                std::cout << "Error: Type mismatch. Key '" << tokens[i] << "' is a [" << current_type << "].\n";
                i += 2; continue;
            }
        }

        // Validation finale des délimiteurs pour types numériques/booléens
        if (!is_bare(val_delim) && (forced == KIVA_TYPE_NUMBER || forced == KIVA_TYPE_BOOLEAN)) {
             std::cout << "Error: Numbers and Booleans must not be quoted.\n";
             i += 2; continue;
        }

        kiva_set_ex(*db, tokens[i].c_str(), tokens[i+1].c_str(), forced, 0);
        std::cout << "OK: " << tokens[i] << " updated.\n";
        i += 2;
    }
}

void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and" || tokens[i] == "string" || tokens[i] == "number" || tokens[i] == "boolean") 
            continue;
        
        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted. Use bare text or ``.\n";
            continue;
        }

        char* res = kiva_get(*db, tokens[i].c_str());
        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
        if (res) free(res);
    }
}

void handle_change(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    (void)delimiters;
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
        } else { i++; }
    }
}

void handle_typeof(KivaDB** db, const std::vector<std::string>& tokens) {
    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        std::cout << " -> " << tokens[i] << " is a [" << kiva_typeof(*db, tokens[i].c_str()) << "]\n";
    }
}

void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path) {
    if (tokens.size() >= 3 && tokens[1] == "all" && tokens[2] == "keys") {
        kiva_close(*db); 
        if (remove(db_path) == 0) {
            *db = kiva_open(db_path); 
            std::cout << "Database cleared and re-initialized successfully.\n";
        } else {
            std::cout << "Error: Could not delete database file.\n";
        }
    } 
    else {
        for (size_t i = 1; i < tokens.size(); i++) {
            if (tokens[i] == "and" || tokens[i] == "string" || tokens[i] == "number" || tokens[i] == "boolean") 
                continue;
            if (kiva_delete(*db, tokens[i].c_str()) == KIVA_OK) {
                std::cout << "Deleted: " << tokens[i] << "\n";
            } else {
                std::cout << "Not found: " << tokens[i] << "\n";
            }
        }
    }
}