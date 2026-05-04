#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdint.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

int64_t kiva_get_file_size(const char* path) {
    FILE* fp = fopen(path, "rb");
    if (!fp) return 0;
    fseek(fp, 0L, SEEK_END);
    int64_t size = ftell(fp);
    fclose(fp);
    return size;
}

// Ici l'ajout de kiva_lock_file et kiva_unlock_file
// pour la gestion multi-processus plus tard.