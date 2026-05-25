/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "index_internal.hpp"

extern "C" {

/**
 * Initialise l'index en mémoire.
 */
void index_init(KivaDB* db) {
    if (db) db->cpp_index = new KivaIndex();
}

/**
 * Libère la mémoire de l'index.
 */
void index_free(KivaDB* db) {
    if (db && db->cpp_index) {
        delete static_cast<KivaIndex*>(db->cpp_index);
        db->cpp_index = nullptr;
    }
}

/**
 * Retourne le chemin vers le fichier de données de la base.
 */
const char* kiva_get_db_path(KivaDB* db) {
    return (db) ? db->path : "Unknown";
}

} // extern "C"