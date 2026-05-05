#include "commands.hpp"
#include <iostream>
#include <cstdlib>
#include <vector>
#include <string>
#include <cstring>
#include <algorithm>

// --- Fonctions utilitaires internes pour la validation ---

static bool is_string_quote(char d) { return d == '"' || d == '\''; }
static bool is_backtick(char d) { return d == '`'; }
static bool is_bare(char d) { return d == 0; }

/**
 * Vérifie si une chaîne est un mot-clé réservé par le système.
 * RÈGLE : Ces mots ne peuvent JAMAIS être utilisés comme noms de clés.
 */
static bool is_reserved_keyword(const std::string& key) {
    static const std::vector<std::string> keywords = {
        "set", "get", "update", "change", "del", "typeof", 
        "scan", "stats", "compact", "exit", "clear", "help",
        "string", "number", "boolean", "and", "ttl", "to", "all", "keys"
    };
    return std::find(keywords.begin(), keywords.end(), key) != keywords.end();
}

/**
 * Gère la commande SET avec validation stricte.
 */
void handle_set(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    int global_ttl = 0;
    
    for (size_t j = 0; j < tokens.size(); j++) {
        if (tokens[j] == "ttl" && j + 1 < tokens.size()) {
            try { global_ttl = std::stoi(tokens[j+1]); } catch(...) { global_ttl = 0; }
        }
    }

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

        // PROTECTION : Mots-clés réservés
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is a reserved keyword and cannot be used as a key.\n";
            i += 2; continue;
        }

        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes. Use bare text or ``.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];

        if (forced == KIVA_TYPE_UNKNOWN) {
            if (is_string_quote(val_delim)) {
                forced = KIVA_TYPE_STRING;
            } 
            else {
                KivaType inferred = kiva_identify_type(val_str.c_str()); 
                if (inferred == KIVA_TYPE_STRING) {
                    std::cout << "Error: String values like '" << val_str << "' must be quoted (\"\" or '').\n";
                    i += 2; continue;
                }
                forced = inferred;
            }
        } 
        else {
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

        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << tokens[i] << "' cannot use backticks.\n";
            i += 2; continue;
        }

        char* exists = kiva_get(*db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' exists. Use 'update'.\n";
            free(exists); i += 2; continue;
        }

        KivaStatus status = kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), forced, global_ttl);
        if (status == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        } else {
            std::cout << "Error: Could not save '" << tokens[i] << "'.\n";
        }
        
        i += 2;
    }
}

/**
 * Gère UPDATE avec protection des mots-clés.
 */
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    for (size_t i = 1; i + 1 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        if (i + 1 >= tokens.size()) break;

        // Pas besoin de vérifier is_reserved_keyword ici car SET l'aura déjà bloqué, 
        // mais on le laisse par sécurité si on modifie le comportement futur.
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is reserved.\n";
            i += 2; continue;
        }

        const char* current_type = kiva_typeof(*db, tokens[i].c_str());
        if (strcmp(current_type, "none") == 0) {
            std::cout << "Error: Key '" << tokens[i] << "' not found.\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];

        if (forced == KIVA_TYPE_UNKNOWN) {
            if (is_string_quote(val_delim)) {
                forced = KIVA_TYPE_STRING;
            } else {
                KivaType inferred = kiva_identify_type(val_str.c_str());
                if (inferred == KIVA_TYPE_STRING) {
                    std::cout << "Error: String values must be quoted.\n";
                    i += 2; continue;
                }
                forced = inferred;
            }
        }

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

        kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), forced, 0);
        std::cout << "OK: " << tokens[i] << " updated.\n";
        i += 2;
    }
}

/**
 * Gère GET avec protection et reset de type.
 */
void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "string") { requested_type = KIVA_TYPE_STRING; continue; }
        if (tokens[i] == "number") { requested_type = KIVA_TYPE_NUMBER; continue; }
        if (tokens[i] == "boolean") { requested_type = KIVA_TYPE_BOOLEAN; continue; }
        if (tokens[i] == "and") continue;
        
        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' is quoted.\n";
            requested_type = KIVA_TYPE_UNKNOWN;
            continue;
        }

        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());
        
        if (requested_type != KIVA_TYPE_UNKNOWN) {
            if (strcmp(actual_type_str, "none") == 0) {
                std::cout << tokens[i] << ": (nil)\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }

            std::string actual(actual_type_str);
            bool mismatch = (requested_type == KIVA_TYPE_STRING && actual != "string") ||
                            (requested_type == KIVA_TYPE_NUMBER && actual != "number") ||
                            (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean");

            if (mismatch) {
                std::cout << "Error: Type mismatch. '" << tokens[i] << "' is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }
        }

        char* res = kiva_get(*db, tokens[i].c_str());
        std::cout << tokens[i] << ": " << (res ? res : "(nil)") << "\n";
        if (res) free(res);
        requested_type = KIVA_TYPE_UNKNOWN;
    }
}

/**
 * Gère CHANGE avec protection de la cible.
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    (void)delimiters;
    for (size_t i = 1; i + 2 < tokens.size(); ) {
        if (tokens[i] == "and") { i++; continue; }

        if (tokens[i+1] == "to") {
            if (is_reserved_keyword(tokens[i+2])) {
                std::cout << "Error: Target '" << tokens[i+2] << "' is a reserved keyword.\n";
                i += 3; continue;
            }

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
            std::cout << "Database cleared successfully.\n";
        } else {
            std::cout << "Error: Could not delete database file.\n";
        }
        return;
    } 

    KivaType requested_type = KIVA_TYPE_UNKNOWN;

    for (size_t i = 1; i < tokens.size(); i++) {
        // 1. Détection du type de sécurité
        if (tokens[i] == "string") { requested_type = KIVA_TYPE_STRING; continue; }
        if (tokens[i] == "number") { requested_type = KIVA_TYPE_NUMBER; continue; }
        if (tokens[i] == "boolean") { requested_type = KIVA_TYPE_BOOLEAN; continue; }
        if (tokens[i] == "and") continue;

        // 2. Vérification du type avant suppression
        const char* actual_type_str = kiva_typeof(*db, tokens[i].c_str());

        if (requested_type != KIVA_TYPE_UNKNOWN) {
            std::string actual(actual_type_str);
            
            // Si la clé n'existe pas, on sort proprement
            if (actual == "none") {
                std::cout << "Not found: " << tokens[i] << "\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }

            bool mismatch = false;
            if (requested_type == KIVA_TYPE_STRING && actual != "string") mismatch = true;
            if (requested_type == KIVA_TYPE_NUMBER && actual != "number") mismatch = true;
            if (requested_type == KIVA_TYPE_BOOLEAN && actual != "boolean") mismatch = true;

            if (mismatch) {
                std::cout << "Error: Type mismatch. Cannot delete '" << tokens[i] 
                          << "' because it is a [" << actual_type_str << "].\n";
                requested_type = KIVA_TYPE_UNKNOWN;
                continue;
            }
        }

        // 3. Suppression effective
        if (kiva_delete(*db, tokens[i].c_str()) == KIVA_OK) {
            std::cout << "Deleted: " << tokens[i] << "\n";
        } else {
            std::cout << "Not found: " << tokens[i] << "\n";
        }
        
        requested_type = KIVA_TYPE_UNKNOWN; // Reset pour la clé suivante
    }
}