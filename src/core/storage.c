#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

// Chargement au démarrage
static void kiva_load_index(KivaDB* db) {
    fseek(db->file, 0, SEEK_SET);
    uint32_t k_size, v_size;

    while (fread(&k_size, sizeof(uint32_t), 1, db->file) == 1) {
        if (fread(&v_size, sizeof(uint32_t), 1, db->file) != 1) break;
        char* key = malloc(k_size + 1);
        fread(key, 1, k_size, db->file);
        key[k_size] = '\0';

        if (v_size == 0) {
            index_remove(db, key);
            free(key);
        } else {
            index_set(db, key, ftell(db->file), v_size);
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

    kiva_load_index(db);
    return db;
}

KivaStatus kiva_set(KivaDB* db, const char* key, const char* value) {
    uint32_t k_size = strlen(key), v_size = strlen(value);
    fseek(db->file, 0, SEEK_END);
    long pos = ftell(db->file);

    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fwrite(value, 1, v_size, db->file);
    fflush(db->file);

    index_set(db, key, pos + (sizeof(uint32_t) * 2) + k_size, v_size);
    return KIVA_OK;
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
    uint32_t k_size = strlen(key), v_size = 0;
    fseek(db->file, 0, SEEK_END);
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fflush(db->file);
    index_remove(db, key);
    return KIVA_OK;
}

void kiva_close(KivaDB* db) {
    if (!db) return;
    kiva_unlock_file(db->file);
    // (Ajouter ici une boucle pour libérer les HashNodes si besoin)
    fclose(db->file);
    free(db->path); free(db);
}

KivaStatus kiva_compact(KivaDB* db) {
    if (!db) return KIVA_ERR_NOT_FOUND;

    char temp_path[256];
    snprintf(temp_path, sizeof(temp_path), "%s.tmp", db->path);
    FILE* temp_file = fopen(temp_path, "wb");
    if (!temp_file) return FILE_ERR_WRITE;

    printf("Compacting database...\n");

    // Parcourir toute la Hash Map
    for (int i = 0; i < HASH_SIZE; i++) {
        HashNode* node = db->index[i];
        while (node) {
            // Lire la valeur actuelle dans l'ancien fichier
            char* val = malloc(node->entry.v_size + 1);
            fseek(db->file, node->entry.offset, SEEK_SET);
            fread(val, 1, node->entry.v_size, db->file);

            // Écrire dans le nouveau fichier (format binaire classique)
            uint32_t k_size = (uint32_t)strlen(node->key);
            uint32_t v_size = node->entry.v_size;
            
            long new_offset_start = ftell(temp_file);
            fwrite(&k_size, sizeof(uint32_t), 1, temp_file);
            fwrite(&v_size, sizeof(uint32_t), 1, temp_file);
            fwrite(node->key, 1, k_size, temp_file);
            fwrite(val, 1, v_size, temp_file);

            // Mettre à jour l'index en RAM avec la nouvelle position
            node->entry.offset = new_offset_start + (sizeof(uint32_t) * 2) + k_size;

            free(val);
            node = node->next;
        }
    }

    // Fermer et remplacer les fichiers
    fclose(db->file);
    fclose(temp_file);

    remove(db->path);           // Supprime l'ancien gros fichier
    rename(temp_path, db->path); // Renomme le petit fichier propre

    // Réouvrir le fichier proprement
    db->file = fopen(db->path, "ab+");
    kiva_lock_file(db->file);

    printf("Compaction finished successfully.\n");
    return KIVA_OK;
}