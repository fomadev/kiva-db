#include "../commands.hpp"
#include <iostream>
#include <cstring>

/**
 * Gère le renommage de clés existantes.
 * Supporte le chaînage : change <old> to <new> and <old2> to <new2>
 */
void handle_change(KivaDB** db, const std::vector<std::string>& tokens) {
    // 1. Validation minimale de la taille pour une opération simple
    if (tokens.size() < 4) {
        std::cout << "Usage: change <old_key> to <new_key> [and <old2> to <new2>...]\n";
        return;
    }

    // 2. Boucle de traitement pour gérer le chaînage avec 'and'
    for (size_t i = 1; i + 2 < tokens.size(); ) {
        // Ignorer le mot-clé de liaison "and"
        if (tokens[i] == "and") { 
            i++; 
            continue; 
        }

        // On vérifie la présence du mot-clé "to" (ex: change a TO b)
        if (tokens[i + 1] == "to") {
            std::string old_key = tokens[i];
            std::string new_key = tokens[i + 2];

            // A. Protection contre les mots-clés réservés pour la destination
            if (is_reserved_keyword(new_key)) {
                std::cout << "Error: Target '" << new_key << "' is a reserved keyword.\n";
                i += 3; continue;
            }

            // B. Vérification de l'existence de la source
            const char* type_old = kiva_typeof(*db, old_key.c_str());
            if (std::strcmp(type_old, "none") == 0 || std::strcmp(type_old, "undefined") == 0) {
                std::cout << "Error: Source '" << old_key << "' not found.\n";
                i += 3; continue;
            }

            // C. Vérification de collision (la destination ne doit pas exister)
            const char* type_new = kiva_typeof(*db, new_key.c_str());
            if (std::strcmp(type_new, "none") != 0 && std::strcmp(type_new, "undefined") != 0) {
                std::cout << "Error: Target '" << new_key << "' already exists.\n";
                i += 3; continue;
            }

            // D. Exécution du renommage via l'API C
            if (kiva_rename(*db, old_key.c_str(), new_key.c_str()) == KIVA_OK) {
                std::cout << "OK: " << old_key << " -> " << new_key << "\n";
            } else {
                std::cout << "Error: Failed to rename '" << old_key << "'.\n";
            }

            i += 3; // On avance de 3 (old + to + new)
        } else {
            // Si la syntaxe est incorrecte (ex: manque le "to")
            std::cout << "Error: Expected 'to' after '" << tokens[i] << "'.\n";
            break;
        }
    }
}