/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <iomanip>
#include <vector>
#include <string>

// Déclarations des fonctions de l'API externe (core/api.c et index/index_telemetry.cpp)
uint32_t index_get_count(const KivaDB* db);
size_t index_get_live_size(const KivaDB* db); // Renvoie la somme des tailles des clés + valeurs actives
const char* kiva_get_db_path(const KivaDB* db);
long long kiva_get_file_size(const char* path);
size_t kiva_get_memory_usage(const KivaDB* db);

/**
 * Affiche les statistiques détaillées de la base de données (v2.1.8).
 * Calcule et affiche le taux de fragmentation dû à l'architecture Append-Only.
 */
void handle_stats(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    // Éviter les avertissements de compilation pour les paramètres inutilisés
    (void)tokens; 
    (void)delimiters;

    if (!db || !*db) {
        std::cout << "Error: Database not initialized.\n";
        return;
    }

    // 1. Récupération des métriques matérielles et logiques
    uint32_t keys = index_get_count(*db);
    const char* path = kiva_get_db_path(*db); 
    long long f_size = kiva_get_file_size(path);       // Taille physique sur disque (Size_disk)
    size_t mem_usage = kiva_get_memory_usage(*db);     // Empreinte mémoire totale de l'index
    size_t live_size = index_get_live_size(*db);       // Taille utile des données vivantes (Size_live)

    // 2. Calcul mathématique du ratio de fragmentation
    double frag_ratio = 0.0;
    if (f_size > 0 && static_cast<unsigned long long>(f_size) > live_size) {
        frag_ratio = (1.0 - (static_cast<double>(live_size) / static_cast<double>(f_size))) * 100.0;
    }

    // 3. Affichage du Dashboard (Formatage Kitoko-Style)
    std::cout << "\n" << std::setfill('=') << std::setw(42) << "" << std::endl;
    std::cout << "     KivaDB Engine Statistics (v2.1.8)" << std::endl;
    std::cout << std::setfill('-') << std::setw(42) << "" << std::setfill(' ') << std::endl;

    // Métrique : Nombre de clés
    std::cout << " > Total Keys          : " << keys << std::endl;

    // Métrique : Taille du fichier sur le Disque (Storage)
    std::cout << " > Storage Size        : ";
    if (f_size < 1024) {
        std::cout << f_size << " bytes" << std::endl;
    } else if (f_size < 1024 * 1024) {
        std::cout << std::fixed << std::setprecision(2) << (f_size / 1024.0) << " KB" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (f_size / (1024.0 * 1024.0)) << " MB" << std::endl;
    }

    // Métrique : Consommation RAM globale
    std::cout << " > Memory Usage (RAM)  : ";
    if (mem_usage < 1024) {
        std::cout << mem_usage << " bytes" << std::endl;
    } else if (mem_usage < 1024 * 1024) {
        std::cout << std::fixed << std::setprecision(2) << (mem_usage / 1024.0) << " KB" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (mem_usage / (1024.0 * 1024.0)) << " MB" << std::endl;
    }

    // Métrique : Taux de fragmentation (Append-Only overhead)
    std::cout << " > Disk Fragmentation  : " << std::fixed << std::setprecision(1) << frag_ratio << "%";
    if (frag_ratio >= 20.0) {
        std::cout << " [Action Needed: 'compact']";
    }
    std::cout << std::endl;

    // Métrique : Emplacement physique
    std::cout << " > Database Path       : " << (path ? path : "N/A") << std::endl;

    std::cout << std::setfill('=') << std::setw(42) << "" << std::setfill(' ') << "\n" << std::endl;
}