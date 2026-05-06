#include "commands.hpp"
#include <iostream>
#include <vector>
#include <string>
#include <algorithm>

/**
 * NOTE : Les fonctions handle_set, handle_get, handle_update, handle_del 
 * et handle_change sont maintenant définies dans le dossier src/cli/handle/
 * et liées lors de la compilation.
 */

/**
 * Gère la commande TYPEOF (Relais simple car la logique est courte)
 */
void handle_typeof(KivaDB** db, const std::vector<std::string>& tokens) {
    if (tokens.size() < 2) {
        std::cout << "Usage: typeof <key1> and <key2>...\n";
        return;
    }

    for (size_t i = 1; i < tokens.size(); i++) {
        if (tokens[i] == "and") continue;
        
        const char* type = kiva_typeof(*db, tokens[i].c_str());
        std::cout << " -> " << tokens[i] << " is a [" << type << "]\n";
    }
}

/**
 * Gère SCAN (Affiche toutes les clés avec leurs métadonnées)
 */
void handle_scan(KivaDB** db) {
    kiva_scan(*db);
}

/**
 * Gère STATS (Statistiques de stockage)
 */
void handle_stats(KivaDB** db) {
    kiva_stats(*db);
}

/**
 * Gère COMPACT (Réorganisation du fichier de stockage)
 */
void handle_compact(KivaDB** db) {
    std::cout << "Compacting database...\n";
    kiva_compact(db);
    std::cout << "Compactation terminée.\n";
}