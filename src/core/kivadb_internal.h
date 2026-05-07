#ifndef KIVADB_INTERNAL_H
#define KIVADB_INTERNAL_H

#include <stdio.h>
#include <stdint.h>
#include "../../include/kivadb.h"

/* 
 * Le bloc extern "C" est vital ici : il permet à index.cpp (C++) et storage.c (C)
 * de partager ces prototypes sans que le C++ ne change le nom des fonctions (Name Mangling).
 */
#ifdef __cplusplus
extern "C" {
#endif

/**
 * Structure interne de la base de données.
 * cpp_index est un pointeur opaque vers la structure KivaIndex définie dans index.cpp.
 */
struct KivaDB {
    FILE* file;
    char* path;
    void* cpp_index; 
};

// --- Prototypes de gestion de l'Index (Implémentés dans index.cpp) ---

void index_init(KivaDB* db);
void index_free(KivaDB* db);

// Définit une entrée standard (TTL = 0)
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type);

// Définit une entrée avec support du TTL
void index_set_ex(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type, int ttl_sec);

// Recherche une clé (gère la Lazy Deletion si expiré)
int  index_lookup(KivaDB* db, const char* key, KeyDirEntry* out_entry);

// Supprime une clé de l'index
void index_remove(KivaDB* db, const char* key);

// Affiche l'état de l'index (Debug)
void index_scan(KivaDB* db);

// Retourne le nombre total de clés actives
int  index_get_count(KivaDB* db);


// --- Fonctions de Format et Type (Implémentées dans format_v2.c) ---

// Détecte automatiquement le type (String, Number, Boolean) d'une chaîne
KivaType detect_type(const char* value);

int is_valid_number(const char* s);
int is_valid_boolean(const char* s);

// Détecte si le fichier est en format V1 ou V2
int kiva_detect_format(FILE* file);

// Charge l'index en mémoire à l'ouverture
void kiva_load_index(KivaDB* db);


// --- Fonctions de Compaction et Système ---

// Effectue la migration des données valides vers le fichier temporaire
void kiva_internal_compact_step(KivaDB* db, FILE* temp_file);

// Verrouillage de fichier pour éviter les accès concurrents
int  kiva_lock_file(FILE* file);
void kiva_unlock_file(FILE* file);


const char* kiva_get_db_path(KivaDB* db);

#ifdef __cplusplus
}
#endif

#endif