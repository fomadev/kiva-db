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
        // Si le TTL est dépassé, on supprime de l'index et on fait comme si la clé n'existait pas
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

    for (auto it = map.begin(); it != map.end(); ) {
        if (it->second.expires_at > 0 && it->second.expires_at < now) {
            it = map.erase(it); 
            continue;
        }

        char* val_ptr = (char*)malloc(it->second.v_size + 1);
        fseek(db->file, it->second.offset, SEEK_SET);
        fread(val_ptr, 1, it->second.v_size, db->file);
        val_ptr[it->second.v_size] = '\0';
        
        valid_entries.push_back({it->first, std::string(val_ptr), it->second.type, it->second.expires_at});
        free(val_ptr);
        ++it;
    }

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

    std::cout << "\n--- KivaDB Scan (v2.1.1 STL with TTL support) ---\n";
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

/* --- Nouvelles fonctions pour l'API STATS (v2.1.1) --- */

/**
 * Retourne le nombre exact de clés indexées.
 */
uint32_t index_get_count(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;
    return (uint32_t)static_cast<KivaIndex*>(db->cpp_index)->map.size();
}

/**
 * Calcule l'usage mémoire approximatif de l'index en RAM.
 * (Estimation : Taille des clés + taille des structures KeyDirEntry)
 */
size_t kiva_get_memory_usage(KivaDB* db) {
    if (!db || !db->cpp_index) return 0;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    size_t total = sizeof(KivaIndex);
    for (const auto& [key, entry] : map) {
        total += key.capacity(); // Taille du string
        total += sizeof(KeyDirEntry); // Taille de la valeur d'index
        total += 32; // Overhang approximatif pour les nodes de l'unordered_map
    }
    return total;
}

/**
 * Retourne le chemin de la base de données.
 */
const char* kiva_get_path(KivaDB* db) {
    return (db) ? db->db_path : "Unknown";
}

} // extern "C"