#ifndef KIVADB_H
#define KIVADB_H

#define KIVADB_VERSION "1.1.0"
#define MAGIC_SIGNATURE "KIVA"
#define FORMAT_V1 1  // Old format (no header)
#define FORMAT_V2 2  // New format (with header and types)

#include <stddef.h>
#include <stdint.h>

/* Cross-platform directory creation macros */
#ifdef _WIN32
    #include <direct.h>
    #define MKDIR(dir) _mkdir(dir)
#else
    #include <sys/stat.h>
    #include <sys/types.h>
    #define MKDIR(dir) mkdir(dir, 0755)
#endif

// Structure opaque pour masquer les détails d'implémentation
typedef struct KivaDB KivaDB;

// File header for portability and format versioning
typedef struct {
    char signature[4];        // "KIVA"
    uint32_t format_version;  // FORMAT_V1 or FORMAT_V2
    uint32_t reserved;        // For future extensions
} KivaHeader;

// Codes de retour pour la gestion d'erreurs
typedef enum {
    KIVA_OK = 0,
    KIVA_NOT_FOUND = -1,
    KIVA_WRITE_ERROR = -2,
    KIVA_ERR_OPEN,
    FILE_ERR_WRITE,
    KIVA_ERR_NOT_FOUND,
    KIVA_ERR_MALLOC,
    KIVA_ERR_LEGACY_FORMAT
} KivaStatus;

// Types de données supportés par KivaDB
typedef enum {
    KIVA_TYPE_STRING = 1,
    KIVA_TYPE_NUMBER = 2,
    KIVA_TYPE_BOOLEAN = 3,
    KIVA_TYPE_UNKNOWN = 0
} KivaType;

// Entrée de l'index en mémoire
typedef struct {
    int64_t offset;      // Position fixe pour la portabilité
    uint32_t v_size;     // Taille de la valeur
    KivaType type;       // Type de donnée
} KeyDirEntry;

/**
 * Ouvre ou crée une base de données KivaDB.
 * @param path Chemin vers le fichier .kiva
 * @return Un pointeur vers l'instance KivaDB ou NULL en cas d'échec.
 */
KivaDB* kiva_open(const char* path);

/**
 * Stocke une paire clé-valeur avec détection automatique du type.
 */
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value);

/**
 * Stocke une paire clé-valeur avec un type explicitement forcé.
 * Nouveau dans la version 1.1.0
 */
KivaStatus kiva_set_with_type(KivaDB* db, const char* key, const char* value, KivaType forced_type);

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
 * Ferme la base de données et libère la mémoire.
 */
void kiva_close(KivaDB* db);

/**
 * Réorganise la base de données et met à jour le format vers FORMAT_V2.
 */
KivaStatus kiva_compact(KivaDB* db);

/**
 * Retourne la taille du fichier de base de données sur le disque.
 */
int64_t kiva_get_file_size(const char* path);

/**
 * Retourne le type de donnée sous forme de chaîne (string, number, boolean).
 */
const char* kiva_typeof(KivaDB* db, const char* key);

#endif // KIVADB_H