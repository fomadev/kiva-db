/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <iomanip>
#include <ctime>
#include <vector>

/**
 * Gère la commande SCAN.
 * Affiche l'intégralité des clés avec leurs métadonnées (Type, Taille, Timestamp).
 */
void handle_scan(KivaDB** db, const std::vector<std::string>& tokens) {
    (void)tokens; // Empêche le warning unused parameter
    if (!db || !*db) return;

    // Récupération du nombre de clés via l'API publique
    uint32_t count = index_get_count(*db);
    
    std::cout << "\n--- KivaDB Scan : " << count << " active keys ---\n";
    std::cout << std::left << std::setw(18) << " KEY" 
              << " | " << std::setw(10) << "TYPE" 
              << " | " << std::setw(10) << "SIZE" 
              << " | " << "CREATED AT" << "\n";
    std::cout << std::string(75, '-') << "\n";

    if (count == 0) {
        std::cout << " (empty database)\n";
    } else {
        // Appelle la fonction de scan du moteur (déclarée dans commands.hpp)
        index_scan(*db);
    }
    
    std::cout << std::string(75, '-') << "\n\n";
}