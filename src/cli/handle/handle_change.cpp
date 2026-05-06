#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère le renommage d'une clé existante.
 * Syntaxe : change <old_key> to <new_key>
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens) {
    // 1. Validation de la syntaxe
    if (tokens.size() < 4 || tokens[2] != "to") {
        std::cout << "Usage: change <old_key> to <new_key>\n";
        return;
    }

    std::string old_key = tokens[1];
    std::string new_key = tokens[3];

    // 2. Vérification des mots-clés réservés pour la nouvelle clé
    if (is_reserved_keyword(new_key)) {
        std::cout << "Error: '" << new_key << "' is a reserved keyword and cannot be used as a key name.\n";
        return;
    }

    // 3. Vérification de l'existence de la clé source
    const char* type_old = kiva_typeof(*db, old_key.c_str());
    if (std::strcmp(type_old, "none") == 0) {
        std::cout << "Error: Source key '" << old_key << "' does not exist.\n";
        return;
    }

    // 4. Vérification de collision (la destination ne doit pas déjà exister)
    const char* type_new = kiva_typeof(*db, new_key.c_str());
    if (std::strcmp(type_new, "none") != 0) {
        std::cout << "Error: Target key '" << new_key << "' already exists. Use 'update' or 'del' first.\n";
        return;
    }

    // 5. Exécution du renommage via l'API
    if (kiva_rename(*db, old_key.c_str(), new_key.c_str()) == KIVA_OK) {
        std::cout << "OK: '" << old_key << "' renamed to '" << new_key << "'.\n";
    } else {
        std::cout << "Error: Failed to rename key.\n";
    }
}