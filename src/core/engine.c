/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

/**
 * Ouvre la base de données et charge l'index en mémoire.
 */
KivaDB* kiva_open(const char* path) {
    KivaDB* db = (KivaDB*)calloc(1, sizeof(KivaDB));
    if (!db) return NULL;
    
    // Initialise la table de hachage C++ via le wrapper interne
    index_init(db);
    db->path = strdup(path);

    // Vérification de l'existence pour savoir s'il faut créer le header
    FILE* check = fopen(path, "rb");
    int exists = (check != NULL);
    if (check) fclose(check);

    // Ouverture en mode "append" (ab+) pour garantir que les écritures se font à la fin
    db->file = fopen(path, "ab+");
    if (!db->file) {
        index_free(db);
        free(db->path);
        free(db);
        return NULL;
    }

    // Si le fichier est nouveau ou vide, on écrit le Header V2
    if (!exists || kiva_get_file_size(path) == 0) {
        KivaHeader header;
        memcpy(header.signature, MAGIC_SIGNATURE, 4); 
        header.format_version = FORMAT_V2;
        header.reserved = 0;
        fwrite(&header, sizeof(KivaHeader), 1, db->file);
        fflush(db->file);
    }

    // Optimisation des performances disque (Buffer de 64Ko)
    setvbuf(db->file, NULL, _IOFBF, 65536);
    
    // Chargement de l'index depuis le disque vers la mémoire
    kiva_load_index(db);
    
    return db;
}

/**
 * Ferme proprement la base et libère la mémoire.
 */
void kiva_close(KivaDB* db) {
    if (!db) return;
    
    kiva_unlock_file(db->file);
    fclose(db->file);
    index_free(db); // Libère la HashTable C++
    free(db->path); 
    free(db);
}

/**
 * Réinitialise complètement la base de données (Tronquage).
 * Résout le problème de lenteur après suppression massive.
 */
KivaStatus kiva_reset(KivaDB* db) {
    if (!db) return KIVA_ERR_NOT_FOUND;

    // 1. Fermer le fichier actuel et le réouvrir en mode "wb+" 
    // Cela efface instantanément tout le contenu du fichier (taille = 0).
    fclose(db->file);
    db->file = fopen(db->path, "wb+"); 
    if (!db->file) return FILE_ERR_WRITE;

    // 2. Réécrire l'en-tête V2 immédiatement pour que le fichier reste valide
    KivaHeader header;
    memcpy(header.signature, MAGIC_SIGNATURE, 4);
    header.format_version = FORMAT_V2;
    header.reserved = 0;
    fwrite(&header, sizeof(KivaHeader), 1, db->file);
    fflush(db->file);

    // 3. VIDER L'INDEX EN MÉMOIRE
    // Indispensable : sinon le programme croit que les clés existent encore.
    index_free(db);
    index_init(db);

    // 4. Repasser en mode append pour les prochaines écritures
    fclose(db->file);
    db->file = fopen(db->path, "ab+");
    setvbuf(db->file, NULL, _IOFBF, 65536);

    return KIVA_OK;
}

/**
 * Compacte la base de données pour supprimer les données obsolètes (Garbage Collection).
 */
KivaStatus kiva_compact(KivaDB* db) {
    if (!db) return KIVA_ERR_NOT_FOUND;

    char temp_path[256];
    snprintf(temp_path, sizeof(temp_path), "%s.tmp", db->path);
    
    // Création d'un nouveau fichier temporaire propre
    FILE* temp_file = fopen(temp_path, "wb");
    if (!temp_file) return FILE_ERR_WRITE;

    // Écriture du Header V2 dans le temporaire
    KivaHeader header;
    memcpy(header.signature, MAGIC_SIGNATURE, 4);
    header.format_version = FORMAT_V2;
    header.reserved = 0;
    fwrite(&header, sizeof(KivaHeader), 1, temp_file);

    // Migration uniquement des données vivantes (non supprimées / non expirées)
    kiva_internal_compact_step(db, temp_file);

    fclose(db->file);
    fclose(temp_file);

    // Remplacement du vieux fichier par le nouveau compacté
    remove(db->path);
    if (rename(temp_path, db->path) != 0) {
        return FILE_ERR_WRITE;
    }

    // Réouverture de la base compactée
    db->file = fopen(db->path, "ab+");
    kiva_lock_file(db->file);
    setvbuf(db->file, NULL, _IOFBF, 65536);

    return KIVA_OK;
}