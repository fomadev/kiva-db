#include "../commands.hpp"
#include <iostream>
#include <iomanip>

/**
 * Affiche les statistiques détaillées de la base de données.
 * Centralise les informations de stockage (disque) et d'indexation (RAM).
 */
void handle_stats(KivaDB** db) {
    if (!db || !*db) {
        std::cout << "Error: Database not initialized.\n";
        return;
    }

    // 1. Récupération des données via les fonctions de l'API
    uint32_t keys = index_get_count(*db);
    const char* path = kiva_get_db_path(*db); 
    long long f_size = kiva_get_file_size(path);
    size_t mem_usage = kiva_get_memory_usage(*db);

    // 2. Affichage du Dashboard
    std::cout << "\n" << std::setfill('=') << std::setw(38) << "" << std::endl;
    std::cout << "    KivaDB Statistics (v" << KIVADB_VERSION << ")" << std::endl;
    std::cout << std::setfill('-') << std::setw(38) << "" << std::setfill(' ') << std::endl;

    // Affichage des Clés
    std::cout << " > Total Keys         : " << keys << std::endl;

    // Affichage Taille Disque (Storage)
    std::cout << " > Storage Size       : ";
    if (f_size < 1024) {
        std::cout << f_size << " bytes" << std::endl;
    } else if (f_size < 1024 * 1024) {
        std::cout << std::fixed << std::setprecision(2) << (f_size / 1024.0) << " KB" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (f_size / (1024.0 * 1024.0)) << " MB" << std::endl;
    }

    // Affichage Usage RAM (Index)
    std::cout << " > Memory Usage (RAM) : ";
    if (mem_usage < 1024) {
        std::cout << mem_usage << " bytes" << std::endl;
    } else if (mem_usage < 1024 * 1024) {
        std::cout << std::fixed << std::setprecision(2) << (mem_usage / 1024.0) << " KB" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (mem_usage / (1024.0 * 1024.0)) << " MB" << std::endl;
    }

    // Affichage du Chemin
    std::cout << " > Database Path      : " << (path ? path : "N/A") << std::endl;

    std::cout << std::setfill('=') << std::setw(38) << "" << std::setfill(' ') << "\n" << std::endl;
}