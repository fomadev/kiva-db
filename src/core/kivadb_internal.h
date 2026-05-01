#ifndef KIVADB_INTERNAL_H
#define KIVADB_INTERNAL_H

#include <stdio.h>
#include <stdint.h>
#include "../../include/kivadb.h"

#ifdef __cplusplus
extern "C" {
#endif

struct KivaDB {
    FILE* file;
    char* path;
    void* cpp_index; // Pointeur vers KivaIndex (C++ Map)
};

// --- Prototypes Index ---
void index_init(KivaDB* db);
void index_free(KivaDB* db);
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type);
void index_remove(KivaDB* db, const char* key);
void index_scan(KivaDB* db);
int index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry);
int index_get_count(KivaDB* db);

// --- Système ---
int kiva_lock_file(FILE* file);
void kiva_unlock_file(FILE* file);

#ifdef __cplusplus
}
#endif

#endif