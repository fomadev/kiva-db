#include "kivadb_internal.h"
#include <unordered_map>
#include <string>
#include <iostream>
#include <iomanip>

// On définit une structure pour encapsuler la Map C++ proprement
struct KivaIndex {
    std::unordered_map<std::string, KeyDirEntry> map;
};

extern "C" {

void index_init(KivaDB* db) {
    if (db) db->cpp_index = new KivaIndex();
}

void index_free(KivaDB* db) {
    if (db && db->cpp_index) {
        delete static_cast<KivaIndex*>(db->cpp_index);
        db->cpp_index = nullptr;
    }
}

void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type) {
    if (!db || !db->cpp_index || !key) return;
    
    auto* index = static_cast<KivaIndex*>(db->cpp_index);
    KeyDirEntry entry = {offset, v_size, type};
    index->map[std::string(key)] = entry;
}

void index_remove(KivaDB* db, const char* key) {
    if (!db || !db->cpp_index || !key) return;
    static_cast<KivaIndex*>(db->cpp_index)->map.erase(key);
}

// Nouvelle fonction indispensable pour kiva_get en C
int index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry) {
    if (!db || !db->cpp_index || !key) return 0;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;
    auto it = map.find(key);
    if (it != map.end()) {
        *out_entry = it->second;
        return 1;
    }
    return 0;
}

void index_scan(KivaDB* db) {
    if (!db || !db->cpp_index) return;
    auto& map = static_cast<KivaIndex*>(db->cpp_index)->map;

    std::cout << "\n--- KivaDB Scan (v2.0.0 STL) ---\n";
    for (const auto& [key, entry] : map) {
        std::string t = (entry.type == KIVA_TYPE_NUMBER) ? "number" : 
                        (entry.type == KIVA_TYPE_BOOLEAN) ? "boolean" : "string";
        std::cout << "  -> " << std::left << std::setw(15) << key 
                  << " | " << std::setw(8) << t 
                  << " | " << entry.v_size << " bytes\n";
    }
    std::cout << "Total: " << map.size() << " keys.\n--------------------------------\n";
}

int index_get_count(KivaDB* db) {
    return db && db->cpp_index ? static_cast<KivaIndex*>(db->cpp_index)->map.size() : 0;
}
}