/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "index_internal.hpp"
#include <vector>
#include <cstdlib>
#include <ctime>

extern "C" {

/**
 * kiva_internal_compact_step : Le cœur du nettoyage physique.
 */
void kiva_internal_compact_step(KivaDB* db, FILE* temp_file) {
    if (!db || !db->cpp_index || !temp_file) return;
    
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    time_t now = std::time(nullptr);
    
    struct ValidEntry { 
        std::string key; 
        std::string val; 
        KivaType type; 
        int64_t expires_at; 
        int64_t timestamp;
    };
    std::vector<ValidEntry> valid_entries;

    // Phase 1 : Extraction des données valides du fichier actuel
    for (auto it = map.begin(); it != map.end(); ) {
        if (it->second.expires_at > 0 && it->second.expires_at < (int64_t)now) {
            it = map.erase(it); 
            continue;
        }

        char* val_ptr = (char*)std::malloc(it->second.v_size + 1);
        if (!val_ptr) { ++it; continue; }
        
        std::fseek(db->file, it->second.offset, SEEK_SET);
        std::fread(val_ptr, 1, it->second.v_size, db->file);
        val_ptr[it->second.v_size] = '\0';
        
        valid_entries.push_back({
            it->first, 
            std::string(val_ptr), 
            (KivaType)it->second.type, 
            it->second.expires_at, 
            it->second.timestamp
        });
        std::free(val_ptr);
        ++it;
    }

    // Phase 2 : Réécriture compacte dans le nouveau fichier temp_file
    for (const auto& e : valid_entries) {
        uint32_t k_size = (uint32_t)e.key.length();
        uint32_t v_size = (uint32_t)e.val.length();
        uint8_t t_byte = (uint8_t)e.type;
        int64_t exp = e.expires_at;
        int64_t ts = e.timestamp;

        long pos = std::ftell(temp_file);
        std::fwrite(&k_size, sizeof(uint32_t), 1, temp_file);
        std::fwrite(&v_size, sizeof(uint32_t), 1, temp_file);
        std::fwrite(&t_byte, sizeof(uint8_t), 1, temp_file);
        std::fwrite(&exp, sizeof(int64_t), 1, temp_file);
        std::fwrite(&ts, sizeof(int64_t), 1, temp_file); 
        std::fwrite(e.key.c_str(), 1, k_size, temp_file);
        std::fwrite(e.val.c_str(), 1, v_size, temp_file);

        // Calcul du nouvel offset pointant vers la valeur
        int64_t header_size = (sizeof(uint32_t) * 2) + sizeof(uint8_t) + (sizeof(int64_t) * 2);
        int64_t new_offset = (int64_t)(pos + header_size + k_size);
        
        KeyDirEntry updated;
        updated.offset = new_offset;
        updated.v_size = v_size;
        updated.type = (KivaType)e.type;
        updated.expires_at = exp;
        updated.timestamp = ts;

        map[e.key] = updated;
    }
}

} // extern "C"