#include "kivadb_internal.h"
#include <unordered_map>
#include <string>
#include <iostream>
#include <iomanip>
#include <ctime>

// Structure pour encapsuler la Map C++ proprement
struct KivaIndex {
    std::unordered_map<std::string, KeyDirEntry> map;
};

extern "C" {

/**
 * Initialise l'index en mémoire
 */
void index_init(KivaDB* db) {
    if (db) db->cpp_index = new KivaIndex();
}

/**
 * Libère la mémoire de l'index
 */
void index_free(KivaDB* db) {
    if (db && db->cpp_index) {
        delete static_cast<KivaIndex*>(db->cpp_index);
        db->cpp_index = nullptr;
    }
}

/**
 * Définit ou met à jour une entrée sans TTL (TTL = 0)
 */
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type) {
    if (!db || !db->cpp_index || !key) return;
    
    auto* index = static_cast<KivaIndex*>(db->cpp_index);
    // On initialise expires_at à 0 pour indiquer "pas d'expiration"
    KeyDirEntry entry = {offset, v_size, type, 0};
    index->map[std::string(key)] = entry;
}

/**
 * Définit ou met à jour une entrée avec un TTL (Time To Live)
 */
void index_set_ex(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type, int ttl_sec) {
    if (!db || !db->cpp_index || !key) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    
    int64_t expiry = (ttl_sec > 0) ? (static_cast<int64_t>(std::time(nullptr)) + ttl_sec) : 0;
    map[std::string(key)] = {offset, v_size, type, expiry};
}

/**
 * Recherche une clé avec gestion de la suppression paresseuse (Lazy Deletion) si expiré
 */
int index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry) {
    if (!db || !db->cpp_index || !key) return 0;
    
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    auto it = map.find(key);

    if (it != map.end()) {
        // Vérification du TTL (si expires_at > 0, on compare au temps actuel)
        if (it->second.expires_at > 0 && it->second.expires_at < std::time(nullptr)) {
            map.erase(it); // Suppression "à la volée" car expiré
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
 * Supprime manuellement une clé de l'index
 */
void index_remove(KivaDB* db, const char* key) {
    if (!db || !db->cpp_index || !key) return;
    static_cast<KivaIndex*>(db->cpp_index)->map.erase(key);
}

/**
 * Affiche le contenu de l'index (utile pour le debug)
 */
void index_scan(KivaDB* db) {
    if (!db || !db->cpp_index) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    time_t now = std::time(nullptr);

    std::cout << "\n--- KivaDB Scan (v2.0.0 STL with TTL support) ---\n";
    for (const auto& [key, entry] : map) {
        // Vérification sommaire pour l'affichage du statut expiré
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

/**
 * Retourne le nombre d'éléments dans l'index
 */
int index_get_count(KivaDB* db) {
    return (db && db->cpp_index) ? static_cast<KivaIndex*>(db->cpp_index)->map.size() : 0;
}

} // extern "C"