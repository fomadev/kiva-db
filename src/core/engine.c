#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

KivaDB* kiva_open(const char* path) {
    KivaDB* db = (KivaDB*)calloc(1, sizeof(KivaDB));
    if (!db) return NULL;
    
    index_init(db);
    db->path = strdup(path);

    FILE* check = fopen(path, "rb");
    int exists = (check != NULL);
    if (check) fclose(check);

    db->file = fopen(path, "ab+");
    if (!db->file) return NULL;

    if (!exists || kiva_get_file_size(path) == 0) {
        KivaHeader header;
        memcpy(header.signature, MAGIC_SIGNATURE, 4); 
        header.format_version = FORMAT_V2;
        header.reserved = 0;
        fwrite(&header, sizeof(KivaHeader), 1, db->file);
        fflush(db->file);
    }

    setvbuf(db->file, NULL, _IOFBF, 65536);
    kiva_load_index(db);
    return db;
}

void kiva_close(KivaDB* db) {
    if (!db) return;
    kiva_unlock_file(db->file);
    fclose(db->file);
    index_free(db);
    free(db->path); 
    free(db);
}

KivaStatus kiva_compact(KivaDB* db) {
    if (!db) return KIVA_ERR_NOT_FOUND;

    char temp_path[256];
    snprintf(temp_path, sizeof(temp_path), "%s.tmp", db->path);
    FILE* temp_file = fopen(temp_path, "wb");
    if (!temp_file) return FILE_ERR_WRITE;

    KivaHeader header;
    memcpy(header.signature, MAGIC_SIGNATURE, 4);
    header.format_version = FORMAT_V2;
    header.reserved = 0;
    fwrite(&header, sizeof(KivaHeader), 1, temp_file);

    kiva_internal_compact_step(db, temp_file);

    fclose(db->file);
    fclose(temp_file);

    remove(db->path);
    rename(temp_path, db->path);

    db->file = fopen(db->path, "ab+");
    kiva_lock_file(db->file);
    setvbuf(db->file, NULL, _IOFBF, 65536);

    return KIVA_OK;
}