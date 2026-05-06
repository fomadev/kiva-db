#include "../commands.hpp"
#include <iostream>

// On déclare les fonctions d'utils pour qu'elles soient visibles ici
bool is_reserved_keyword(const std::string& key);
bool is_string_quote(char d);
bool is_bare(char d);
bool is_backtick(char d);

/**
 * Gère la commande SET avec support du TTL, du typage forcé et du chaînage 'and'.
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
        // Ignorer les mots de liaison et les paramètres déjà traités
        if (tokens[i] == "and" || tokens[i] == "ttl") { 
            i += (tokens[i] == "ttl" ? 2 : 1); 
            continue; 
        }
        
        // Détection d'un type forcé (ex: set string ma_cle "123")
        KivaType forced = KIVA_TYPE_UNKNOWN;
        if (tokens[i] == "string") { forced = KIVA_TYPE_STRING; i++; }
        else if (tokens[i] == "number") { forced = KIVA_TYPE_NUMBER; i++; }
        else if (tokens[i] == "boolean") { forced = KIVA_TYPE_BOOLEAN; i++; }

        // Vérification de la présence d'une valeur après la clé
        if (i + 1 >= tokens.size()) {
            std::cout << "Error: Key '" << tokens[i] << "' is missing a value.\n";
            break;
        }

        // --- VALIDATION DE LA CLÉ ---
        // Une clé ne doit pas être un mot réservé (get, set, del, etc.)
        if (is_reserved_keyword(tokens[i])) {
            std::cout << "Error: '" << tokens[i] << "' is a reserved keyword and cannot be used as a key.\n";
            i += 2; continue;
        }

        // Une clé ne doit pas être entourée de guillemets ("" ou '')
        if (is_string_quote(delimiters[i])) {
            std::cout << "Error: Key '" << tokens[i] << "' cannot use quotes. Use bare text or backticks (``).\n";
            i += 2; continue;
        }

        char val_delim = delimiters[i+1];
        std::string val_str = tokens[i+1];

        // --- VALIDATION DU TYPE ET DU FORMAT DE LA VALEUR ---
        if (forced == KIVA_TYPE_UNKNOWN) {
            // Si pas de type forcé, on détecte selon les délimiteurs ou le contenu
            if (is_string_quote(val_delim)) {
                forced = KIVA_TYPE_STRING;
            } 
            else {
                KivaType inferred = kiva_identify_type(val_str.c_str()); 
                // Si c'est du texte pur sans guillemets, on refuse (sauf si c'est un nombre/booléen)
                if (inferred == KIVA_TYPE_STRING) {
                    std::cout << "Error: String values like '" << val_str << "' must be quoted (\"\" or '').\n";
                    i += 2; continue;
                }
                forced = inferred;
            }
        } 
        else {
            // Validation si l'utilisateur a forcé un type
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

        // Protection contre l'usage des backticks sur les valeurs
        if (is_backtick(val_delim)) {
            std::cout << "Error: Value for '" << tokens[i] << "' cannot use backticks.\n";
            i += 2; continue;
        }

        // --- VÉRIFICATION D'EXISTENCE (Anti-Overwrite) ---
        char* exists = kiva_get(*db, tokens[i].c_str());
        if (exists) {
            std::cout << "Error: Key '" << tokens[i] << "' already exists. Use 'update' to change it.\n";
            free(exists); 
            i += 2; continue;
        }

        // --- PERSISTENCE ---
        KivaStatus status = kiva_set_ex(*db, tokens[i].c_str(), val_str.c_str(), forced, global_ttl);
        if (status == KIVA_OK) {
            std::cout << "OK: " << tokens[i] << " saved.\n";
        } else {
            std::cout << "Error: Could not save '" << tokens[i] << "' (Internal error).\n";
        }
        
        i += 2; // Passer à la paire suivante
    }
}