#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

KivaStatus kiva_set_ex(KivaDB* db, const char* key, const char* value, KivaType forced_type, int ttl_sec) {
    if (!db || !key || !value) return KIVA_ERR_NOT_FOUND;

    uint32_t k_size = (uint32_t)strlen(key);
    uint32_t v_size = (uint32_t)strlen(value);
    KivaType type = (forced_type == KIVA_TYPE_UNKNOWN) ? detect_type(value) : forced_type;
    uint8_t type_byte = (uint8_t)type;

    int64_t expires_at = (ttl_sec > 0) ? ((int64_t)time(NULL) + ttl_sec) : 0;

    fseek(db->file, 0, SEEK_END);
    int64_t pos = ftell(db->file);

    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(&expires_at, sizeof(int64_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fwrite(value, 1, v_size, db->file);
    fflush(db->file);

    int64_t value_offset = pos + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + sizeof(int64_t) + k_size;
    index_set_ex(db, key, value_offset, v_size, type, ttl_sec);

    return KIVA_OK;
}

KivaStatus kiva_set(KivaDB* db, const char* key, const char* value) {
    return kiva_set_ex(db, key, value, KIVA_TYPE_UNKNOWN, 0);
}

char* kiva_get(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    if (index_lookup(db, key, &entry)) {
        char* val = (char*)malloc(entry.v_size + 1);
        if (!val) return NULL;
        fseek(db->file, entry.offset, SEEK_SET);
        fread(val, 1, entry.v_size, db->file);
        val[entry.v_size] = '\0';
        return val;
    }
    return NULL;
}

KivaStatus kiva_delete(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    if (!index_lookup(db, key, &entry)) return KIVA_ERR_NOT_FOUND;

    uint32_t k_size = strlen(key), v_size = 0;
    int64_t expires_at = 0;
    fseek(db->file, 0, SEEK_END);
    
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    uint8_t type_byte = (uint8_t)KIVA_TYPE_UNKNOWN;
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(&expires_at, sizeof(int64_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fflush(db->file);

    index_remove(db, key);
    return KIVA_OK;
}

const char* kiva_typeof(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    if (index_lookup(db, key, &entry)) {
        switch(entry.type) {
            case KIVA_TYPE_STRING:  return "string";
            case KIVA_TYPE_NUMBER:  return "number";
            case KIVA_TYPE_BOOLEAN: return "boolean";
            default:                return "unknown";
        }
    }
    return "undefined";
}