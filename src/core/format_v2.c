#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

KivaType detect_type(const char* value) {
    if (!value || *value == '\0') return KIVA_TYPE_STRING;
    
    if (strcmp(value, "true") == 0 || strcmp(value, "false") == 0) {
        return KIVA_TYPE_BOOLEAN;
    }
    
    char* endptr = NULL;
    strtod(value, &endptr);
    if (*value != '\0' && *endptr == '\0') {
        return KIVA_TYPE_NUMBER;
    }
    
    return KIVA_TYPE_STRING;
}

int kiva_detect_format(FILE* file) {
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    if (file_size == 0) return FORMAT_V2; 
    
    char signature[4];
    if (fread(signature, 1, 4, file) == 4 && 
        strncmp(signature, MAGIC_SIGNATURE, 4) == 0) {
        fseek(file, 0, SEEK_SET);
        return FORMAT_V2;
    }
    
    fseek(file, 0, SEEK_SET);
    return FORMAT_V1;
}

void kiva_load_index(KivaDB* db) {
    fseek(db->file, 0, SEEK_SET);
    int format = kiva_detect_format(db->file);
    
    if (format == FORMAT_V2) {
        KivaHeader header;
        if (fread(&header, sizeof(KivaHeader), 1, db->file) != 1) return; 
        if (header.format_version == FORMAT_V1) {
            fprintf(stderr, "[Warning] Old format detected. Run 'compact' to upgrade.\n");
        }
    }
    
    uint32_t k_size, v_size;
    uint8_t type_raw;
    int64_t expires_at = 0;
    
    while (fread(&k_size, sizeof(uint32_t), 1, db->file) == 1) {
        if (fread(&v_size, sizeof(uint32_t), 1, db->file) != 1) break;
        
        if (format == FORMAT_V2) {
            if (fread(&type_raw, sizeof(uint8_t), 1, db->file) != 1) break;
            if (fread(&expires_at, sizeof(int64_t), 1, db->file) != 1) break;
        } else {
            type_raw = KIVA_TYPE_STRING;
            expires_at = 0;
        }
        
        char* key = (char*)malloc(k_size + 1);
        if (!key) break;
        fread(key, 1, k_size, db->file);
        key[k_size] = '\0';
        
        if (v_size == 0) {
            index_remove(db, key);
        } else {
            if (expires_at > 0 && expires_at < (int64_t)time(NULL)) {
                fseek(db->file, v_size, SEEK_CUR);
            } else {
                int64_t current_offset = ftell(db->file);
                index_set_ex(db, key, current_offset, v_size, (KivaType)type_raw, 
                            (expires_at > 0) ? (int)(expires_at - time(NULL)) : 0);
                fseek(db->file, v_size, SEEK_CUR);
            }
        }
        free(key);
    }
}