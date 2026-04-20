#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

/**
 * Intelligent type inference for data migration and compact operations.
 * Detects Boolean, Number (int/float), and String types automatically.
 */
static KivaType detect_type(const char* value) {
    if (!value || *value == '\0') {
        return KIVA_TYPE_STRING;
    }
    
    // Check for boolean values
    if (strcmp(value, "true") == 0 || strcmp(value, "false") == 0) {
        return KIVA_TYPE_BOOLEAN;
    }
    
    // Check for numeric values (int or float)
    char* endptr = NULL;
    strtod(value, &endptr);
    // If entire string was consumed by number parser, it's a number
    if (*value != '\0' && *endptr == '\0') {
        return KIVA_TYPE_NUMBER;
    }
    
    return KIVA_TYPE_STRING;
}

/**
 * Detect file format: FORMAT_V2 (with header) or FORMAT_V1 (legacy, no header)
 */
static int kiva_detect_format(FILE* file) {
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    if (file_size == 0) {
        return FORMAT_V2;  // Empty file defaults to new format
    }
    
    // Try to read header signature
    char signature[4];
    if (fread(signature, 1, 4, file) == 4 && 
        strncmp(signature, MAGIC_SIGNATURE, 4) == 0) {
        fseek(file, 0, SEEK_SET);
        return FORMAT_V2;
    }
    
    // No header found, it's legacy format
    fseek(file, 0, SEEK_SET);
    return FORMAT_V1;
}

/**
 * Load index from file with support for both FORMAT_V1 and FORMAT_V2
 */
static void kiva_load_index(KivaDB* db) {
    fseek(db->file, 0, SEEK_SET);
    int format = kiva_detect_format(db->file);
    
    // Skip header if FORMAT_V2
    if (format == FORMAT_V2) {
        KivaHeader header;
        if (fread(&header, sizeof(KivaHeader), 1, db->file) != 1) {
            return;  // Empty database
        }
        if (header.format_version == FORMAT_V1) {
            fprintf(stderr, "[Warning] Old format detected. Running in legacy mode. "
                            "Please run 'compact' to upgrade to v%s\n", KIVADB_VERSION);
        }
    }
    
    uint32_t k_size, v_size;
    uint8_t type_raw;
    
    while (fread(&k_size, sizeof(uint32_t), 1, db->file) == 1) {
        if (fread(&v_size, sizeof(uint32_t), 1, db->file) != 1) break;
        
        if (format == FORMAT_V2) {
            if (fread(&type_raw, sizeof(uint8_t), 1, db->file) != 1) break;
        } else {
            type_raw = KIVA_TYPE_STRING;
        }
        
        char* key = malloc(k_size + 1);
        if (!key) break;
        fread(key, 1, k_size, db->file);
        key[k_size] = '\0';
        
        if (v_size == 0) {
            index_remove(db, key);
            free(key);
        } else {
            int64_t current_offset = ftell(db->file);
            index_set(db, key, current_offset, v_size, (KivaType)type_raw);
            fseek(db->file, v_size, SEEK_CUR);
            free(key);
        }
    }
}

KivaDB* kiva_open(const char* path) {
    KivaDB* db = calloc(1, sizeof(KivaDB));
    if (!db) return NULL;
    db->path = strdup(path);
    
    db->file = fopen(path, "ab+");
    if (!db->file || kiva_lock_file(db->file) == -1) {
        if (db->file) fclose(db->file);
        free(db->path); free(db);
        return NULL;
    }

    setvbuf(db->file, NULL, _IOFBF, 65536);
    kiva_load_index(db);
    return db;
}

/**
 * Core internal set function that handles writing to disk and indexing
 */
static KivaStatus kiva_internal_set(KivaDB* db, const char* key, const char* value, KivaType type) {
    uint32_t k_size = strlen(key), v_size = strlen(value);
    uint8_t type_byte = (uint8_t)type;
    
    fseek(db->file, 0, SEEK_END);
    int64_t pos = ftell(db->file);
    
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fwrite(value, 1, v_size, db->file);
    fflush(db->file);
    
    int64_t value_offset = pos + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + k_size;
    index_set(db, key, value_offset, v_size, type);
    return KIVA_OK;
}

KivaStatus kiva_set(KivaDB* db, const char* key, const char* value) {
    // Inférence automatique si appelé sans type spécifique
    KivaType type = detect_type(value);
    return kiva_internal_set(db, key, value, type);
}

KivaStatus kiva_set_with_type(KivaDB* db, const char* key, const char* value, KivaType forced_type) {
    // Si le type est UNKNOWN, on utilise l'inférence, sinon on respecte le choix
    KivaType type_to_use = (forced_type == KIVA_TYPE_UNKNOWN) ? detect_type(value) : forced_type;
    return kiva_internal_set(db, key, value, type_to_use);
}

char* kiva_get(KivaDB* db, const char* key) {
    unsigned long h = hash_function(key);
    HashNode* node = db->index[h];
    while (node) {
        if (strcmp(node->key, key) == 0) {
            char* val = malloc(node->entry.v_size + 1);
            fseek(db->file, node->entry.offset, SEEK_SET);
            fread(val, 1, node->entry.v_size, db->file);
            val[node->entry.v_size] = '\0';
            return val;
        }
        node = node->next;
    }
    return NULL;
}

KivaStatus kiva_delete(KivaDB* db, const char* key) {
    unsigned long h = hash_function(key);
    HashNode* node = db->index[h];
    int found = 0;

    while (node) {
        if (strcmp(node->key, key) == 0) {
            found = 1;
            break;
        }
        node = node->next;
    }

    if (!found) {
        return KIVA_ERR_NOT_FOUND;
    }

    uint32_t k_size = strlen(key), v_size = 0;
    fseek(db->file, 0, SEEK_END);
    
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    // On garde l'espace pour le type_byte pour garder le format cohérent
    uint8_t type_byte = (uint8_t)KIVA_TYPE_UNKNOWN;
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    
    fwrite(key, 1, k_size, db->file);
    fflush(db->file);

    index_remove(db, key);
    return KIVA_OK;
}

void kiva_close(KivaDB* db) {
    if (!db) return;
    kiva_unlock_file(db->file);
    fclose(db->file);
    free(db->path); free(db);
}

KivaStatus kiva_compact(KivaDB* db) {
    if (!db) return KIVA_ERR_NOT_FOUND;

    char temp_path[256];
    snprintf(temp_path, sizeof(temp_path), "%s.tmp", db->path);
    FILE* temp_file = fopen(temp_path, "wb");
    if (!temp_file) return FILE_ERR_WRITE;

    printf("Compacting database to v%s format...\n", KIVADB_VERSION);

    KivaHeader header;
    memcpy(header.signature, MAGIC_SIGNATURE, 4);
    header.format_version = FORMAT_V2;
    header.reserved = 0;
    fwrite(&header, sizeof(KivaHeader), 1, temp_file);

    for (int i = 0; i < HASH_SIZE; i++) {
        HashNode* node = db->index[i];
        while (node) {
            char* val = malloc(node->entry.v_size + 1);
            fseek(db->file, node->entry.offset, SEEK_SET);
            fread(val, 1, node->entry.v_size, db->file);

            uint32_t k_size = (uint32_t)strlen(node->key);
            uint32_t v_size = node->entry.v_size;
            uint8_t type_byte = (uint8_t)node->entry.type;
            
            int64_t new_offset_start = ftell(temp_file);
            fwrite(&k_size, sizeof(uint32_t), 1, temp_file);
            fwrite(&v_size, sizeof(uint32_t), 1, temp_file);
            fwrite(&type_byte, sizeof(uint8_t), 1, temp_file);
            fwrite(node->key, 1, k_size, temp_file);
            fwrite(val, 1, v_size, temp_file);

            node->entry.offset = new_offset_start + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + k_size;

            free(val);
            node = node->next;
        }
    }

    fclose(db->file);
    fclose(temp_file);

    remove(db->path);
    rename(temp_path, db->path);

    db->file = fopen(db->path, "ab+");
    kiva_lock_file(db->file);
    setvbuf(db->file, NULL, _IOFBF, 65536);

    printf("Compaction finished successfully. Database upgraded to v%s\n", KIVADB_VERSION);
    return KIVA_OK;
}

int64_t kiva_get_file_size(const char* path) {
    FILE* fp = fopen(path, "rb");
    if (!fp) return 0;
    fseek(fp, 0L, SEEK_END);
    int64_t size = ftell(fp);
    fclose(fp);
    return size;
}

const char* kiva_typeof(KivaDB* db, const char* key) {
    unsigned long h = hash_function(key);
    HashNode* node = db->index[h];
    while (node) {
        if (strcmp(node->key, key) == 0) {
            switch(node->entry.type) {
                case KIVA_TYPE_STRING:   return "string";
                case KIVA_TYPE_NUMBER:   return "number";
                case KIVA_TYPE_BOOLEAN:  return "boolean";
                case KIVA_TYPE_UNKNOWN:
                default:                 return "unknown";
            }
        }
        node = node->next;
    }
    return "undefined";
}