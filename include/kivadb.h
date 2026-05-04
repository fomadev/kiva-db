#ifndef KIVADB_H
#define KIVADB_H

#define KIVADB_VERSION "2.0.1"
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

typedef struct KivaDB KivaDB;

typedef struct {
    char signature[4];
    uint32_t format_version;
    uint32_t reserved;
} KivaHeader;

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

typedef enum {
    KIVA_TYPE_STRING = 1,
    KIVA_TYPE_NUMBER = 2,
    KIVA_TYPE_BOOLEAN = 3,
    KIVA_TYPE_UNKNOWN = 0
} KivaType;

typedef struct {
    int64_t offset;
    uint32_t v_size;
    KivaType type;
    int64_t expires_at; 
} KeyDirEntry;

/* --- API PUBLIQUE --- */

KivaDB* kiva_open(const char* path);
void kiva_close(KivaDB* db);

// Set standard
KivaStatus kiva_set(KivaDB* db, const char* key, const char* value);

// Set avec Type forcé
KivaStatus kiva_set_with_type(KivaDB* db, const char* key, const char* value, KivaType forced_type);

// Set étendu avec Type ET TTL (Utilisé par le Shell)
KivaStatus kiva_set_ex(KivaDB* db, const char* key, const char* value, KivaType forced_type, int ttl_sec);

char* kiva_get(KivaDB* db, const char* key);
KivaStatus kiva_delete(KivaDB* db, const char* key);
KivaStatus kiva_compact(KivaDB* db);
int64_t kiva_get_file_size(const char* path);
const char* kiva_typeof(KivaDB* db, const char* key);

#endif