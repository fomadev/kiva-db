/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "kivadb_internal.h"
#include <unordered_map>
#include <string>
#include <vector>
#include <iostream>
#include <iomanip>
#include <ctime>

/**
 * Structure pour encapsuler la Map C++ proprement.
 * Elle permet d'utiliser la puissance de la STL tout en restant opaque pour le C.
 */
struct KivaIndex {
    std::unordered_map<std::string, KeyDirEntry> map;
};

extern "C" {

/**
 * Initialise l'index en mémoire.
 */
void index_init(KivaDB* db) {
    if (db) db->cpp_index = new KivaIndex();
}

/**
 * Libère la mémoire de l'index.
 */
void index_free(KivaDB* db) {
    if (db && db->cpp_index) {
        delete static_cast<KivaIndex*>(db->cpp_index);
        db->cpp_index = nullptr;
    }
}

/**
 * Définit ou met à jour une entrée sans TTL (TTL = 0).
 */
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type) {
    if (!db || !db->cpp_index || !key) return;
    
    auto* index = static_cast<KivaIndex*>(db->cpp_index);
    KeyDirEntry entry = {offset, v_size, type, 0};
    index->map[std::string(key)] = entry;
}

/**
 * Définit ou met à jour une entrée avec un TTL (Time To Live).
 */
void index_set_ex(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type, int ttl_sec) {
    if (!db || !db->cpp_index || !key) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    int64_t expiry = (ttl_sec > 0) ? (static_cast<int64_t>(std::time(nullptr)) + ttl_sec) : 0;
    map[std::string(key)] = {offset, v_size, type, expiry};
}

/**
 * Recherche une clé avec gestion de la suppression paresseuse (Lazy Deletion).
 */
int index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry) {
    if (!db || !db->cpp_index || !key) return 0;
    
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    auto it = map.find(key);

    if (it != map.end()) {
        // Suppression paresseuse si le TTL est expiré
        if (it->second.expires_at > 0 && it->second.expires_at < std::time(nullptr)) {
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
 * Supprime manuellement une clé de l'index.
 */
void index_remove(KivaDB* db, const char* key) {
    if (!db || !db->cpp_index || !key) return;
    static_cast<KivaIndex*>(db->cpp_index)->map.erase(key);
}

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
    };
    std::vector<ValidEntry> valid_entries;

    // Phase 1 : Extraction des données valides
    for (auto it = map.begin(); it != map.end(); ) {
        if (it->second.expires_at > 0 && it->second.expires_at < now) {
            it = map.erase(it); 
            continue;
        }

        char* val_ptr = (char*)malloc(it->second.v_size + 1);
        if (!val_ptr) { ++it; continue; } // Sécurité mémoire
        
        fseek(db->file, it->second.offset, SEEK_SET);
        fread(val_ptr, 1, it->second.v_size, db->file);
        val_ptr[it->second.v_size] = '\0';
        
        valid_entries.push_back({it->first, std::string(val_ptr), it->second.type, it->second.expires_at});
        free(val_ptr);
        ++it;
    }

    // Phase 2 : Réécriture dans le nouveau fichier et mise à jour de l'index
    for (const auto& e : valid_entries) {
        uint32_t k_size = (uint32_t)e.key.length();
        uint32_t v_size = (uint32_t)e.val.length();
        uint8_t t_byte = (uint8_t)e.type;
        int64_t exp = e.expires_at;

        long pos = ftell(temp_file);
        fwrite(&k_size, sizeof(uint32_t), 1, temp_file);
        fwrite(&v_size, sizeof(uint32_t), 1, temp_file);
        fwrite(&t_byte, sizeof(uint8_t), 1, temp_file);
        fwrite(&exp, sizeof(int64_t), 1, temp_file);
        fwrite(e.key.c_str(), 1, k_size, temp_file);
        fwrite(e.val.c_str(), 1, v_size, temp_file);

        // Calcul de l'offset vers la donnée de valeur pour le prochain index_lookup
        int64_t new_offset = (int64_t)(pos + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + sizeof(int64_t) + k_size);
        
        KeyDirEntry updated_entry = { new_offset, v_size, e.type, exp };
        map[e.key] = updated_entry;
    }
}

/**
 * Affiche l'état actuel de la base (Debug).
 */
void index_scan(KivaDB* db) {
    if (!db || !db->cpp_index) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    time_t now = std::time(nullptr);

    std::cout << "\n--- KivaDB Scan (v2.1.1.5 | FomaDev Public License) ---" << std::endl;
    for (const auto& [key, entry] : map) {
        std::string status = "";
        if (entry.expires_at > 0) {
            if (entry.expires_at < now) status = " [EXPIRED]";
            else status = " [TTL: " + std::to_string(entry.expires_at - now) + "s]";
        }

        std::string t = (entry.type == KIVA_TYPE_NUMBER) ? "number" : 
                        (entry.type == KIVA_TYPE_BOOLEAN) ? "boolean" : "string";
        
        std::cout << "  -> " << std::left << std::setw(15) << key 
                  << " | " << std::setw(8) << t 
                  << " | " << entry.v_size << " bytes" << status << "\n";
    }
    std::cout << "Total: " << map.size() << " keys.\n--------------------------------\n";
}

/* --- Implémentations pour l'API STATS & CHEMIN --- */

/**
 * Retourne le nombre exact de clés indexées.
 */
uint32_t index_get_count(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;
    auto* index = static_cast<KivaIndex*>(db->cpp_index);
    return (uint32_t)index->map.size();
}

/**
 * Calcule l'usage mémoire approximatif de l'index en RAM.
 */
size_t kiva_get_memory_usage(KivaDB* db) {
    if (!db || !db->cpp_index) {
        return 0;
    }

    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    // 1. Taille de base des structures fixes
    size_t total = sizeof(KivaDB) + sizeof(KivaIndex);
    
    // 2. Estimation de la consommation des données dynamiques
    for (auto const& [key, val] : map) {
        // key.capacity() : Mémoire allouée pour le string
        // sizeof(KeyDirEntry) : Structure fixe dans la map
        // 32 : Overhead moyen d'un nœud d'unordered_map (pointeurs next/prev, hash)
        total += key.capacity() + sizeof(KeyDirEntry) + 32; 
    }

    return total;
}

/**
 * Retourne le chemin de la base de données.
 */
const char* kiva_get_db_path(KivaDB* db) {
    return (db) ? db->path : "Unknown";
}

} // extern "C"