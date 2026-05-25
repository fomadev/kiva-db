/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "index_internal.hpp"
#include <iostream>
#include <iomanip>
#include <ctime>

extern "C" {

/**
 * Affiche l'état actuel de la base (Debug / CLI Scan).
 */
void index_scan(KivaDB* db) {
    if (!db || !db->cpp_index) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    time_t now = std::time(nullptr);

    std::cout << "\n--- KivaDB Scan (v2.1.8 | FomaDev Public License) ---" << std::endl;
    for (const auto& [key, entry] : map) {
        time_t raw_time = (time_t)entry.timestamp;
        struct tm* dt = std::localtime(&raw_time);
        char time_buf[20];
        std::strftime(time_buf, sizeof(time_buf), "%Y-%m-%d %H:%M:%S", dt);

        std::string status = "";
        if (entry.expires_at > 0) {
            if (entry.expires_at < (int64_t)now) status = " [EXPIRED]";
            else status = " [TTL: " + std::to_string(entry.expires_at - now) + "s]";
        }

        const char* type_str = (entry.type == KIVA_TYPE_NUMBER) ? "number" : 
                               (entry.type == KIVA_TYPE_BOOLEAN) ? "boolean" : "string";
        
        std::cout << "  -> " << std::left << std::setw(15) << key 
                  << " | " << std::setw(8) << type_str 
                  << " | " << std::setw(7) << entry.v_size << " bytes"
                  << " | " << time_buf << status << "\n";
    }
    std::cout << "Total: " << map.size() << " keys.\n--------------------------------\n" << std::endl;
}

/**
 * Calcule l'usage mémoire approximatif de l'index en RAM.
 */
size_t kiva_get_memory_usage(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;

    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    size_t total = sizeof(KivaDB) + sizeof(KivaIndex);
    
    for (auto const& [key, val] : map) {
        // key.capacity() : Mémoire allouée pour le string
        // sizeof(KeyDirEntry) : Structure fixe
        // 32 : Overhead moyen d'un nœud d'unordered_map
        total += key.capacity() + sizeof(KeyDirEntry) + 32; 
    }

    return total;
}

/**
 * Calcule le volume utile réel (Données vivantes + En-têtes physiques) du fichier disque.
 * Calibré pour correspondre exactement à la structure du format_v2.c.
 */
size_t index_get_live_size(KivaDB* db) {
    // Structure initiale minimale : l'en-tête magique du fichier .kiva (ex: "KIVA" + version)
    size_t total_live_size = 12; 

    if (!db || !db->cpp_index) return total_live_size;

    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    // Taille fixe de l'en-tête binaire de chaque transaction sur disque (Format V2)
    // Contient : timestamp (8B), expires_at (8B), type (1B), k_size (2B), v_size (4B), etc. + padding
    const size_t RECORD_HEADER_OVERHEAD = 24; 

    for (const auto& [key, entry] : map) {
        total_live_size += RECORD_HEADER_OVERHEAD; // Overhead binaire sur disque
        total_live_size += key.size();             // Longueur brute de la clé
        total_live_size += entry.v_size;           // Longueur stockée de la valeur
    }

    return total_live_size;
}

} // extern "C"