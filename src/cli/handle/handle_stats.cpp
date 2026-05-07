#include "../commands.hpp"
#include <iostream>
#include <iomanip>

/**
 * Affiche les statistiques détaillées de la base de données.
 * Utilise les getters de l'API pour l'encapsulation et la cohérence.
 */
void handle_stats(KivaDB** db) {
    if (!db || !*db) {
        std::cerr << "Error: Database is not initialized or closed.\n";
        return;
    }

    // 1. Récupération des données via l'API (déclarée dans kivadb.h)
    uint32_t keys = index_get_count(*db);
    
    // On récupère le chemin via la fonction dédiée car la structure est opaque
    const char* path = kiva_get_db_path(*db); 
    
    long long f_size = kiva_get_file_size(path);
    size_t mem_usage = kiva_get_memory_usage(*db);

    // 2. Affichage formaté (Dashboard style)
    std::cout << "\n" << std::setfill('=') << std::setw(36) << "" << std::endl;
    std::cout << "    KivaDB Statistics (v" << KIVADB_VERSION << ")" << std::endl;
    std::cout << std::setfill('-') << std::setw(36) << "" << std::setfill(' ') << std::endl;

    std::cout << " > Total Keys      : " << keys << std::endl;

    std::cout << " > Storage Size    : ";
    if (f_size < 1024) {
        std::cout << f_size << " bytes" << std::endl;
    } else if (f_size < 1024 * 1024) {
        std::cout << std::fixed << std::setprecision(2) << (f_size / 1024.0) << " KB" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (f_size / (1024.0 * 1024.0)) << " MB" << std::endl;
    }

    std::cout << " > RAM Usage       : ";
    if (mem_usage < 1024) {
        std::cout << mem_usage << " bytes" << std::endl;
    } else {
        std::cout << std::fixed << std::setprecision(2) << (mem_usage / 1024.0) << " KB" << std::endl;
    }

    std::cout << " > Database Path   : " << (path ? path : "N/A") << std::endl;

    std::cout << std::setfill('=') << std::setw(36) << "" << std::setfill(' ') << "\n" << std::endl;
}