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
    
    // Initialisation explicite de tous les champs (évite missing-field-initializers)
    KeyDirEntry entry;
    entry.offset = offset;
    entry.v_size = v_size;
    entry.type = type;
    entry.expires_at = 0;
    entry.timestamp = (int64_t)std::time(nullptr);
    
    index->map[std::string(key)] = entry;
}

/**
 * Définit ou met à jour une entrée avec un TTL (Time To Live).
 */
void index_set_ex(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type, int ttl_sec) {
    if (!db || !db->cpp_index || !key) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    int64_t expiry = (ttl_sec > 0) ? (static_cast<int64_t>(std::time(nullptr)) + ttl_sec) : 0;
    
    KeyDirEntry entry;
    entry.offset = offset;
    entry.v_size = v_size;
    entry.type = type;
    entry.expires_at = expiry;
    entry.timestamp = (int64_t)std::time(nullptr);

    map[std::string(key)] = entry;
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
        int64_t timestamp;
    };
    std::vector<ValidEntry> valid_entries;

    // Phase 1 : Extraction des données valides du fichier actuel
    for (auto it = map.begin(); it != map.end(); ) {
        if (it->second.expires_at > 0 && it->second.expires_at < (int64_t)now) {
            it = map.erase(it); 
            continue;
        }

        char* val_ptr = (char*)malloc(it->second.v_size + 1);
        if (!val_ptr) { ++it; continue; }
        
        fseek(db->file, it->second.offset, SEEK_SET);
        fread(val_ptr, 1, it->second.v_size, db->file);
        val_ptr[it->second.v_size] = '\0';
        
        valid_entries.push_back({
            it->first, 
            std::string(val_ptr), 
            (KivaType)it->second.type, 
            it->second.expires_at, 
            it->second.timestamp
        });
        free(val_ptr);
        ++it;
    }

    // Phase 2 : Réécriture compacte dans le nouveau fichier temp_file
    for (const auto& e : valid_entries) {
        uint32_t k_size = (uint32_t)e.key.length();
        uint32_t v_size = (uint32_t)e.val.length();
        uint8_t t_byte = (uint8_t)e.type;
        int64_t exp = e.expires_at;
        int64_t ts = e.timestamp;

        long pos = ftell(temp_file);
        fwrite(&k_size, sizeof(uint32_t), 1, temp_file);
        fwrite(&v_size, sizeof(uint32_t), 1, temp_file);
        fwrite(&t_byte, sizeof(uint8_t), 1, temp_file);
        fwrite(&exp, sizeof(int64_t), 1, temp_file);
        fwrite(&ts, sizeof(int64_t), 1, temp_file); // On préserve le timestamp d'origine
        fwrite(e.key.c_str(), 1, k_size, temp_file);
        fwrite(e.val.c_str(), 1, v_size, temp_file);

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

/**
 * Affiche l'état actuel de la base (Debug / CLI Scan).
 */
void index_scan(KivaDB* db) {
    if (!db || !db->cpp_index) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    time_t now = std::time(nullptr);

    std::cout << "\n--- KivaDB Scan (v2.1.5 | FomaDev Public License) ---" << std::endl;
    for (const auto& [key, entry] : map) {
        // Conversion du timestamp Unix en format lisible YYYY-MM-DD HH:MM:SS
        time_t raw_time = (time_t)entry.timestamp;
        struct tm* dt = localtime(&raw_time);
        char time_buf[20];
        strftime(time_buf, sizeof(time_buf), "%Y-%m-%d %H:%M:%S", dt);

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

/* --- Implémentations pour l'API STATS & INFOS --- */

/**
 * Retourne le nombre exact de clés indexées.
 */
uint32_t index_get_count(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;
    return (uint32_t)static_cast<KivaIndex*>(db->cpp_index)->map.size();
}

/**
 * Calcule l'usage mémoire approximatif de l'index en RAM.
 */
size_t kiva_get_memory_usage(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;

    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    // 1. Taille de base des structures fixes
    size_t total = sizeof(KivaDB) + sizeof(KivaIndex);
    
    // 2. Estimation de la consommation des données dynamiques
    for (auto const& [key, val] : map) {
        // key.capacity() : Mémoire allouée pour le string (clé)
        // sizeof(KeyDirEntry) : Structure fixe dans la map
        // 32 : Overhead moyen d'un nœud d'unordered_map (pointeurs internes, hash, buckets)
        total += key.capacity() + sizeof(KeyDirEntry) + 32; 
    }

    return total;
}

/**
 * Retourne le chemin vers le fichier de données de la base.
 */
const char* kiva_get_db_path(KivaDB* db) {
    return (db) ? db->path : "Unknown";
}

} // extern "C"