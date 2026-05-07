/* --- Dans src/cli/handle/handle_stats.cpp --- */
#include "../commands.hpp"
#include <iostream>
#include <iomanip>

// Ajout de l'accès à la fonction de chemin si non présente dans commands.hpp
extern "C" const char* kiva_get_db_path(KivaDB* db);

void handle_stats(KivaDB** db) {
    if (!db || !*db) {
        std::cout << "Error: Database not initialized.\n";
        return;
    }

    // 1. Nombre de clés
    uint32_t keys = index_get_count(*db);

    // 2. Taille du fichier (Utilisation de la fonction wrapper pour le chemin)
    const char* path = kiva_get_db_path(*db);
    long long f_size = kiva_get_file_size(path);

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