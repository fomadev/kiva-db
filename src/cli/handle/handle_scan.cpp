/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <iomanip>
#include <vector>
#include <string>

/**
 * Gère la commande SCAN.
 * Cette commande sert d'interface entre l'utilisateur et le moteur d'indexation.
 * Elle affiche l'état complet de la base de données en RAM.
 */
void handle_scan(KivaDB** db, const std::vector<std::string>& tokens) {
    // Empêche le warning unused parameter car SCAN ne prend pas d'arguments
    (void)tokens; 

    if (!db || !*db) {
        std::cerr << "Error: Database not initialized." << std::endl;
        return;
    }

    // Récupération sécurisée du nombre de clés via l'API du core
    uint32_t count = index_get_count(*db);
    
    // En-tête du Shell pour le rendu utilisateur
    std::cout << "\n--- KivaDB Scan (v2.1.6 | FomaDev Public License) ---" << std::endl;
    
    if (count == 0) {
        std::cout << " [!] Database is currently empty." << std::endl;
        std::cout << "-----------------------------------------------------" << std::endl;
        return;
    }

    // Affichage des colonnes de l'en-tête
    std::cout << std::left << std::setw(18) << " KEY" 
              << " | " << std::setw(10) << "TYPE" 
              << " | " << std::setw(12) << "SIZE" 
              << " | " << "CREATED AT / STATUS" << "\n";
    
    // Ligne de séparation visuelle
    std::cout << std::string(75, '-') << "\n";

    /**
     * Appelle la fonction index_scan() située dans src/core/index.cpp.
     * C'est cette fonction qui itère sur la map C++ et affiche chaque entrée
     * avec le formatage du timestamp et du TTL.
     */
    index_scan(*db);

    // Pied de page du scan
    std::cout << "Total active keys: " << count << std::endl;
    std::cout << std::string(75, '-') << "\n" << std::endl;
}