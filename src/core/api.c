#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

/**
 * Définit une clé avec un type forcé et un TTL (Time To Live).
 * Version avec validation stricte des types.
 */
KivaStatus kiva_set_ex(KivaDB* db, const char* key, const char* value, KivaType forced_type, int ttl_sec) {
    // 1. Vérification des entrées (Interdire les valeurs NULL ou vides pour la cohérence)
    if (!db || !key || !value || *value == '\0') {
        return KIVA_ERR_INVALID_INPUT;
    }

    // 2. Validation stricte si un type est spécifié
    if (forced_type == KIVA_TYPE_NUMBER && !is_valid_number(value)) {
        return KIVA_ERR_TYPE_MISMATCH;
    }
    if (forced_type == KIVA_TYPE_BOOLEAN && !is_valid_boolean(value)) {
        return KIVA_ERR_TYPE_MISMATCH;
    }

    // 3. Détermination du type final
    KivaType type = (forced_type == KIVA_TYPE_UNKNOWN) ? detect_type(value) : forced_type;
    uint8_t type_byte = (uint8_t)type;

    uint32_t k_size = (uint32_t)strlen(key);
    uint32_t v_size = (uint32_t)strlen(value);
    
    // Calcul de la date d'expiration (timestamp Unix)
    int64_t expires_at = (ttl_sec > 0) ? ((int64_t)time(NULL) + ttl_sec) : 0;

    // 4. Écriture sur le disque (Append-only)
    fseek(db->file, 0, SEEK_END);
    int64_t pos = ftell(db->file);

    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(&expires_at, sizeof(int64_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fwrite(value, 1, v_size, db->file);
    fflush(db->file);

    // 5. Mise à jour de l'index en mémoire
    // L'offset de la valeur est : position_debut + en-têtes + taille_clé
    int64_t value_offset = pos + (sizeof(uint32_t) * 2) + sizeof(uint8_t) + sizeof(int64_t) + k_size;
    index_set_ex(db, key, value_offset, v_size, type, ttl_sec);

    return KIVA_OK;
}

/**
 * Définit une clé en mode automatique (Type deviné, pas de TTL).
 */
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value) {
    return kiva_set_ex(db, key, value, KIVA_TYPE_UNKNOWN, 0);
}

/**
 * Récupère la valeur d'une clé.
 */
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

/**
 * Supprime une clé en écrivant un marqueur de suppression (Tombstone).
 */
KivaStatus kiva_delete(KivaDB* db, const char* key) {
    KeyDirEntry entry;
    // Si la clé n'existe pas dans l'index, rien à supprimer
    if (!index_lookup(db, key, &entry)) return KIVA_ERR_NOT_FOUND;

    uint32_t k_size = (uint32_t)strlen(key);
    uint32_t v_size = 0; // Taille 0 = Supprimé
    int64_t expires_at = 0;
    uint8_t type_byte = (uint8_t)KIVA_TYPE_UNKNOWN;

    fseek(db->file, 0, SEEK_END);
    
    fwrite(&k_size, sizeof(uint32_t), 1, db->file);
    fwrite(&v_size, sizeof(uint32_t), 1, db->file);
    fwrite(&type_byte, sizeof(uint8_t), 1, db->file);
    fwrite(&expires_at, sizeof(int64_t), 1, db->file);
    fwrite(key, 1, k_size, db->file);
    fflush(db->file);

    // Supprimer de l'index mémoire immédiatement
    index_remove(db, key);
    return KIVA_OK;
}

KivaType kiva_identify_type(const char* value) {
    if (!value) return KIVA_TYPE_STRING;

    // Est-ce un booléen ?
    if (strcmp(value, "true") == 0 || strcmp(value, "false") == 0) {
        return KIVA_TYPE_BOOLEAN;
    }

    // Est-ce un nombre ? (Vérifie si c'est composé de chiffres et d'un seul point)
    char* endptr;
    strtod(value, &endptr);
    if (*endptr == '\0' && endptr != value) {
        return KIVA_TYPE_NUMBER;
    }

    // Par défaut, c'est du texte
    return KIVA_TYPE_STRING;
}

KivaStatus kiva_rename(KivaDB* db, const char* old_key, const char* new_key) {
    if (!db || !old_key || !new_key) return KIVA_ERR_INVALID_INPUT;

    char* value = kiva_get(db, old_key);
    if (!value) return KIVA_ERR_NOT_FOUND;

    // Récupérer le type actuel pour le conserver
    const char* type_str = kiva_typeof(db, old_key);
    KivaType current_type = KIVA_TYPE_STRING;
    if (strcmp(type_str, "number") == 0) current_type = KIVA_TYPE_NUMBER;
    else if (strcmp(type_str, "boolean") == 0) current_type = KIVA_TYPE_BOOLEAN;

    // Créer la nouvelle clé avec les mêmes données
    KivaStatus status = kiva_set_ex(db, new_key, value, current_type, 0);
    
    if (status == KIVA_OK) {
        kiva_delete(db, old_key); // Supprimer l'ancienne
    }

    free(value);
    return status;
}

/**
 * Retourne le nom du type de la donnée pour le CLI.
 */
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