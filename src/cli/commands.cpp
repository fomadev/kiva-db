#include "commands.hpp"
#include <iostream>
#include <vector>
#include <string>
#include <algorithm>
#include <cstring>


/**
 * NOTE : Les handlers (set, get, update, del, change, stats) 
 * sont définis dans le dossier src/cli/handle/
 */
 

/**
 * Gère la commande TYPEOF pour identifier le type de données stocké.
 * Supporte le chaînage via le mot-clé 'and'.
 */
void handle_typeof(KivaDB** db, const std::vector<std::string>& tokens) {
    // Validation minimale : il faut au moins une clé après la commande
    if (tokens.size() < 2) {
        std::cout << "Usage: typeof <key1> [and <key2>...]\n";
        return;
    }

    for (size_t i = 1; i < tokens.size(); i++) {
        // Ignorer le mot-clé de liaison "and" pour permettre typeof a and b
        if (tokens[i] == "and") {
            continue;
        }

        // Appel au moteur C pour récupérer le type (string, number, boolean ou none)
        const char* type = kiva_typeof(*db, tokens[i].c_str());

        // Affichage du résultat
        if (std::strcmp(type, "none") == 0 || std::strcmp(type, "undefined") == 0) {
            std::cout << " -> " << tokens[i] << " does not exist (nil)\n";
        } else {
            std::cout << " -> " << tokens[i] << " is a [" << type << "]\n";
        }
    }
}

/**
 * Gère COMPACT
 */
void handle_compact(KivaDB** db) {
    std::cout << "Compacting database...\n";
    kiva_compact(*db); 
    std::cout << "Compactation terminee.\n";
}