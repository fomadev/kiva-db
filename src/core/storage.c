#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

/**
 * Inférence intelligente des types pour la migration et les opérations de compactage.
 */
static KivaType detect_type(const char* value) {
    if (!value || *value == '\0') {
        return KIVA_TYPE_STRING;
    }
    
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

/**
 * Détecte le format du fichier : FORMAT_V2 (avec header) ou FORMAT_V1 (legacy)
 */
static int kiva_detect_format(FILE* file) {
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    if (file_size == 0) {
        return FORMAT_V2; 
    }
    
    char signature[4];
    if (fread(signature, 1, 4, file) == 4 && 
        strncmp(signature, MAGIC_SIGNATURE, 4) == 0) {
        fseek(file, 0, SEEK_SET);
        return FORMAT_V2;
    }
    
    fseek(file, 0, SEEK_SET);
    return FORMAT_V1;
}

/**
 * Charge l'index depuis le disque avec support du TTL et du format V2
 */
static void kiva_load_index(KivaDB* db) {
    fseek(db->file, 0, SEEK_SET);
    int format = kiva_detect_format(db->file);
    
    if (format == FORMAT_V2) {
        KivaHeader header;
        if (fread(&header, sizeof(KivaHeader), 1, db->file) != 1) {
            return; 
        }
        if (header.format_version == FORMAT_V1) {
            fprintf(stderr, "[Warning] Old format detected. Please run 'compact' to upgrade.\n");
        }
    }
    
    uint32_t k_size, v_size;
    uint8_t type_raw;
    int64_t expires_at = 0;
    
    while (fread(&k_size, sizeof(uint32_t), 1, db->file) == 1) {
        if (fread(&v_size, sizeof(uint32_t), 1, db->file) != 1) break;
        
        if (format == FORMAT_V2) {
            if (fread(&type_raw, sizeof(uint8_t), 1, db->file) != 1) break;
            // On lit le TTL sauvegardé sur disque
            if (fread(&expires_at, sizeof(int64_t), 1, db->file) != 1) break;
        } else {
            type_raw = KIVA_TYPE_STRING;
            expires_at = 0;
        }
        
        char* key = malloc(k_size + 1);
        if (!key) break;
        fread(key, 1, k_size, db->file);
        key[k_size] = '\0';
        
        if (v_size == 0) {
            index_remove(db, key);
        } else {
            // Vérification immédiate de l'expiration au chargement
            if (expires_at > 0 && expires_at < (int64_t)time(NULL)) {
                // On saute la valeur sur le disque sans l'indexer (Lazy Deletion au boot)
                fseek(db->file, v_size, SEEK_CUR);
            } else {
                int64_t current_offset = ftell(db->file);
                // On peuple l'index avec l'expiration disque
                index_set_ex(db, key, current_offset, v_size, (KivaType)type_raw, 
                            (expires_at > 0) ? (int)(expires_at - time(NULL)) : 0);
                fseek(db->file, v_size, SEEK_CUR);
            }
        }
        free(key);
    }
}

KivaDB* kiva_open(const char* path) {
    KivaDB* db = calloc(1, sizeof(KivaDB));
    if (!db) return NULL;
    
    index_init(db);
    db->path = strdup(path);

    FILE* check = fopen(path, "rb");
    int exists = (check != NULL);
    if (check) fclose(check);

    db->file = fopen(path, "ab+");
    if (!db->file) { /* ... erreur ... */ return NULL; }

    if (!exists || kiva_get_file_size(path) == 0) {
        KivaHeader header;
        memcpy(header.signature, "KIVA", 4); 
        header.format_version = FORMAT_V2;
        header.reserved = 0;
        
        fwrite(&header, sizeof(KivaHeader), 1, db->file);
        fflush(db->file);
    }

    setvbuf(db->file, NULL, _IOFBF, 65536);
    kiva_load_index(db);
    return db;
}

/**
 * Version étendue de set incluant le TTL (Time To Live) - Le coeur du stockage
 */
KivaStatus kiva_set_ex(KivaDB* db, const char* key, const char* value, KivaType forced_type, int ttl_sec) {
    if (!db || !key || !value) return KIVA_ERR_NOT_FOUND;

    uint32_t k_size = (uint32_t)strlen(key);
    uint32_t v_size = (uint32_t)strlen(value);
    KivaType type = (forced_type == KIVA_TYPE_UNKNOWN) ? detect_type(value) : forced_type;
    uint8_t type_byte = (uint8_t)type;

    // Calcul du timestamp d'expiration
    int64_t expires_at = 0;
    if (ttl_sec > 0) {
        expires_at = (int64_t)time(NULL) + ttl_sec;
    }

    // Append-Only : On se place à la fin
    fseek(db->file, 0, SEEK_END);
    int64_t pos = ftell(db->file);

    // Écriture du format v2 étendu
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(&expires_at, sizeof(int64_t), 1, db->file); // Nouveau champ disque
    fwrite(key, 1, k_size, db->file);
    fwrite(value, 1, v_size, db->file);
    fflush(db->file);

    // Calcul de l'offset de la valeur (pos + headers + key)
    int64_t value_offset = pos + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + sizeof(int64_t) + k_size;
    
    // Mise à jour de l'index C++ (Lazy expiration gérée là-bas)
    index_set_ex(db, key, value_offset, v_size, type, ttl_sec);

    return KIVA_OK;
}

/**
 * Rétrocompatibilité : kiva_set appelle kiva_set_ex sans TTL
 */
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value) {
    return kiva_set_ex(db, key, value, KIVA_TYPE_UNKNOWN, 0);
}

KivaStatus kiva_set_with_type(KivaDB* db, const char* key, const char* value, KivaType forced_type) {
    return kiva_set_ex(db, key, value, forced_type, 0);
}

char* kiva_get(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    // index_lookup gère la suppression automatique si le TTL est expiré
    if (index_lookup(db, key, &entry)) {
        char* val = malloc(entry.v_size + 1);
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
    if (!index_lookup(db, key, &entry)) {
        return KIVA_ERR_NOT_FOUND;
    }

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

    // Migration des entrées valides via la passerelle C++
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

int64_t kiva_get_file_size(const char* path) {
    FILE* fp = fopen(path, "rb");
    if (!fp) return 0;
    fseek(fp, 0L, SEEK_END);
    int64_t size = ftell(fp);
    fclose(fp);
    return size;
}

const char* kiva_typeof(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    if (index_lookup(db, key, &entry)) {
        switch(entry.type) {
            case KIVA_TYPE_STRING:   return "string";
            case KIVA_TYPE_NUMBER:   return "number";
            case KIVA_TYPE_BOOLEAN:  return "boolean";
            default:                 return "unknown";
        }
    }
    return "undefined";
}