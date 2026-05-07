#include "../commands.hpp"
#include <iostream>
#include <iomanip>

// Déclarations des fonctions externes pour faire le lien avec le moteur C
extern "C" {
    const char* kiva_get_path(KivaDB* db);
    uint32_t index_get_count(KivaDB* db);
    long long kiva_get_file_size(const char* path);
    size_t kiva_get_memory_usage(KivaDB* db);
}

/**
 * Gère la commande STATS
 */
void handle_stats(KivaDB** db) {
    if (!db || !*db) {
        std::cout << "Error: Database not initialized.\n";
        return;
    }

    // 1. Nombre de clés
    uint32_t keys = index_get_count(*db);

    // 2. Taille du fichier 
    // Utilisation du path récupéré via le getter pour obtenir la taille
    long long f_size = kiva_get_file_size(kiva_get_path(*db)); 

    // 3. Usage mémoire
    size_t mem_usage = kiva_get_memory_usage(*db);

    std::cout << "--- KivaDB Statistics (v2.1.1) ---\n";
    std::cout << " > Total Keys    : " << keys << "\n";
    
    if (f_size < 1024) {
        std::cout << " > Storage Size  : " << f_size << " bytes\n";
    } else {
        std::cout << " > Storage Size  : " << std::fixed << std::setprecision(2) 
                  << (f_size / 1024.0) << " KB\n";
    }

    if (mem_usage < 1024) {
        std::cout << " > Memory Usage  : " << mem_usage << " bytes\n";
    } else {
        std::cout << " > Memory Usage  : " << std::fixed << std::setprecision(2) 
                  << (mem_usage / 1024.0) << " KB\n";
    }
    
    std::cout << "----------------------------------\n";
}