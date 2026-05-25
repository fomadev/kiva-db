/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "index_internal.hpp"
#include <ctime>

extern "C" {

/**
 * Définit ou met à jour une entrée sans TTL (TTL = 0).
 */
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type) {
    if (!db || !db->cpp_index || !key) return;
    
    auto* index = static_cast<KivaIndex*>(db->cpp_index);
    
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
 * Supprime manuellement une clé de l'index.
 */
void index_remove(KivaDB* db, const char* key) {
    if (!db || !db->cpp_index || !key) return;
    static_cast<KivaIndex*>(db->cpp_index)->map.erase(key);
}

/**
 * Exécute l'opération BUMP sur une clé existante.
 * mode: 1 pour 'add' (prolongation), 2 pour 'set' (réinitialisation)
 * Retourne true si la clé existait et a été mise à jour, false sinon.
 */
bool index_bump(KivaDB* db, const char* key, int mode, int64_t ttl_sec) {
    if (!db || !db->cpp_index || !key) return false;
    
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    auto it = map.find(key);
    
    if (it == map.end()) return false;
    if (it->second.expires_at > 0 && it->second.expires_at < (int64_t)std::time(nullptr)) {
        map.erase(it);
        return false;
    }

    int64_t now = (int64_t)std::time(nullptr);

    if (mode == 1) { 
        if (it->second.expires_at == 0) {
            it->second.expires_at = now + ttl_sec;
        } else {
            it->second.expires_at += ttl_sec;
        }
    } 
    else if (mode == 2) { 
        it->second.expires_at = now + ttl_sec;
    }

    // TODO: Écrire le changement dans le fichier d'archivage .kiva pour la persistance
    // kiva_write_bump_journal(db, key, it->second.expires_at);

    return true;
}

} // extern "C"