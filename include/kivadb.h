#ifndef KIVADB_H
#define KIVADB_H

#define KIVADB_VERSION "1.0.3"

#include <stddef.h>
#include <stdint.h>

// Structure opaque pour masquer les détails d'implémentation à l'utilisateur
typedef struct KivaDB KivaDB;

// Codes de retour pour la gestion d'erreurs
typedef enum {
    KIVA_OK = 0,
    KIVA_NOT_FOUND = -1,
    KIVA_WRITE_ERROR = -2,
    KIVA_ERR_OPEN,
    FILE_ERR_WRITE,
    KIVA_ERR_NOT_FOUND,
    KIVA_ERR_MALLOC
} KivaStatus;

typedef enum {
    TYPE_STRING = 0,
    TYPE_NUMBER = 1,
    TYPE_BOOLEAN = 2
} KivaType;

typedef struct {
    long offset;
    uint32_t v_size;
    KivaType type; // Vérifie bien que cette ligne est présente ici
} KeyDirEntry;

/**
 * Ouvre ou crée une base de données KivaDB.
 * @param path Chemin vers le fichier .kiva
 * @return Un pointeur vers l'instance KivaDB ou NULL en cas d'échec.
 */
KivaDB* kiva_open(const char* path);

/**
 * Stocke une paire clé-valeur. Si la clé existe, elle est mise à jour (Append).
 */
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value);

/**
 * Récupère une valeur. 
 * Note: L'utilisateur doit libérer (free) la mémoire retournée.
 */
char* kiva_get(KivaDB* db, const char* key);

/**
 * Supprime une clé de la base de données.
 */
KivaStatus kiva_delete(KivaDB* db, const char* key);

/**
 * Ferme la base de données et libère la mémoire de l'index.
 */
void kiva_close(KivaDB* db);

KivaStatus kiva_compact(KivaDB* db);

long kiva_get_file_size(const char* path);

const char* kiva_typeof(KivaDB* db, const char* key);
#endif // KIVADB_H