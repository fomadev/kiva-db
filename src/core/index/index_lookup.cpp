/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "index_internal.hpp"
#include <ctime>

extern "C" {

/**
 * Recherche une clé avec gestion de la suppression paresseuse (Lazy Deletion).
 */
int index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry) {
    if (!db || !db->cpp_index || !key) return 0;
    
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    auto it = map.find(key);

    if (it != map.end()) {
        if (it->second.expires_at > 0 && it->second.expires_at < (int64_t)std::time(nullptr)) {
            map.erase(it);
            return 0;
        }
        
        if (out_entry) {
            *out_entry = it->second;
        }
        return 1;
    }
    return 0;
}

/**
 * Retourne le nombre exact de clés indexées.
 */
uint32_t index_get_count(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;
    return (uint32_t)static_cast<KivaIndex*>(db->cpp_index)->map.size();
}

} // extern "C"