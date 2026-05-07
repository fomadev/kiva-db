#include "../commands.hpp"
#include <iostream>
#include <iomanip>

/**
 * Gère la commande STATS (Statistiques de stockage et mémoire)
 * Affiche : Nombre de clés, Taille du fichier, Usage mémoire
 */
void handle_stats(KivaDB** db) {
    if (!db || !*db) {
        std::cout << "Error: Database not initialized.\n";
        return;
    }

    // 1. Nombre de clés
    uint32_t keys = index_get_count(*db);

    // 2. Taille du fichier sur le disque
    long long f_size = kiva_get_file_size((*db)->path);

    // 3. Usage approximatif de la mémoire vive (RAM)
    size_t mem_usage = kiva_get_memory_usage(*db);

    std::cout << "--- KivaDB Statistics (v2.1.1) ---\n";
    std::cout << " > Total Keys    : " << keys << "\n";
    
    // Formatage de la taille du stockage
    if (f_size < 1024) {
        std::cout << " > Storage Size  : " << f_size << " bytes\n";
    } else {
        std::cout << " > Storage Size  : " << std::fixed << std::setprecision(2) 
                  << (f_size / 1024.0) << " KB\n";
    }

    // Formatage de l'usage mémoire
    if (mem_usage < 1024) {
        std::cout << " > Memory Usage  : " << mem_usage << " bytes\n";
    } else {
        std::cout << " > Memory Usage  : " << std::fixed << std::setprecision(2) 
                  << (mem_usage / 1024.0) << " KB\n";
    }
    
    std::cout << "----------------------------------\n";
}