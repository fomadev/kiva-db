/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#ifndef KIVADB_H
#define KIVADB_H

#define KIVADB_VERSION "2.1.1.5"
#define MAGIC_SIGNATURE "KIVA"
#define FORMAT_V1 1
#define FORMAT_V2 2

#include <stddef.h>
#include <stdint.h>

/* Macros de création de dossier cross-platform */
#ifdef _WIN32
    #include <direct.h>
    #define MKDIR(dir) _mkdir(dir)
#else
    #include <sys/stat.h>
    #include <sys/types.h>
    #define MKDIR(dir) mkdir(dir, 0755)
#endif

/* 
 * Pour assurer la compatibilité C++, on englobe les déclarations 
 * si le header est inclus dans un fichier .cpp
 */
#ifdef __cplusplus
extern "C" {
#endif

typedef struct KivaDB KivaDB;

/**
 * Structure de l'en-tête du fichier de base de données (V2)
 */
typedef struct {
    char signature[4];        // "KIVA"
    uint32_t format_version;  // 1 ou 2
    uint32_t reserved;        // Aligné pour usage futur
} KivaHeader;

/**
 * Statuts de retour pour les opérations de l'API
 */
typedef enum {
    KIVA_OK = 0,
    KIVA_ERR_NOT_FOUND = 1,
    FILE_ERR_OPEN = 2,
    FILE_ERR_WRITE = 3,
    KIVA_ERR_INVALID_INPUT = 4,  // Erreur : Entrée NULL ou vide
    KIVA_ERR_TYPE_MISMATCH = 5,   // Erreur : Valeur non conforme au type forcé
    KIVA_ERR_MALLOC = 6,
    KIVA_ERR_LEGACY_FORMAT = 7
} KivaStatus;

/**
 * Types de données supportés par KivaDB
 */
typedef enum {
    KIVA_TYPE_UNKNOWN = 0,
    KIVA_TYPE_STRING = 1,
    KIVA_TYPE_NUMBER = 2,
    KIVA_TYPE_BOOLEAN = 3
} KivaType;

/**
 * Entrée de l'index en mémoire (KeyDir)
 */
typedef struct {
    int64_t offset;      // Position de la valeur dans le fichier
    uint32_t v_size;     // Taille de la valeur
    KivaType type;       // Type de donnée
    int64_t expires_at;  // Timestamp Unix d'expiration (0 si infini)
} KeyDirEntry;

/* --- API PUBLIQUE --- */

/**
 * Initialisation et Fermeture
 */
KivaDB* kiva_open(const char* path);
void kiva_close(KivaDB* db);
KivaStatus kiva_reset(KivaDB* db); 

/**
 * Opérations de Stockage
 */
// Set standard (Type auto-détecté)
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value);

// Set étendu avec Type ET TTL (utilisé par le Shell)
KivaStatus kiva_set_ex(KivaDB* db, const char* key, const char* value, KivaType forced_type, int ttl_sec);

/**
 * Opérations de Lecture et Suppression
 */
char* kiva_get(KivaDB* db, const char* key);
KivaStatus kiva_delete(KivaDB* db, const char* key);
const char* kiva_typeof(KivaDB* db, const char* key);
KivaStatus kiva_rename(KivaDB* db, const char* old_key, const char* new_key);

/**
 * Maintenance et Utilitaires
 */
KivaStatus kiva_compact(KivaDB* db);
int64_t kiva_get_file_size(const char* path);
void kiva_scan(KivaDB* db);
void kiva_stats(KivaDB* db);

/**
 * Getters et Introspection (Indispensables pour STATS et Shell)
 */
const char* kiva_get_path(KivaDB* db);           // Récupère le chemin du fichier DB
uint32_t index_get_count(KivaDB* db);            // Nombre de clés actives
size_t kiva_get_memory_usage(KivaDB* db);        // Estimation RAM de l'index
KivaType kiva_identify_type(const char* value);  // Analyseur de type de chaîne

const char* kiva_get_db_path(KivaDB* db);

#ifdef __cplusplus
}
#endif

#endif /* KIVADB_H */