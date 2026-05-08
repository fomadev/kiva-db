/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#define _DEFAULT_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <time.h>
#include <ctype.h>
#include "../../include/kivadb.h"
#include "kivadb_internal.h"

// --- Fonctions de Validation Stricte ---

/**
 * Vérifie si une chaîne est un nombre pur (entier ou flottant).
 * Utilise strtod pour une précision maximale et vérifie que toute la chaîne est consommée.
 */
int is_valid_number(const char* s) {
    if (!s || *s == '\0') return 0;
    
    // Ignorer les espaces au début pour la flexibilité de saisie
    while (isspace((unsigned char)*s)) s++;
    if (*s == '\0') return 0;

    char* endptr;
    strtod(s, &endptr);

    // Ignorer les espaces à la fin
    while (isspace((unsigned char)*endptr)) endptr++;

    // Retourne 1 si endptr pointe sur la fin de chaîne (nombre pur)
    return *endptr == '\0';
}

/**
 * Vérifie si une chaîne est un booléen pur (true/false).
 */
int is_valid_boolean(const char* s) {
    if (!s) return 0;
    return (strcmp(s, "true") == 0 || strcmp(s, "false") == 0);
}

/**
 * Analyse la chaîne de caractères pour deviner son type de donnée.
 * Utilisé par le mode automatique quand aucun type n'est forcé.
 */
KivaType detect_type(const char* value) {
    if (!value || *value == '\0') return KIVA_TYPE_STRING;
    
    if (is_valid_boolean(value)) {
        return KIVA_TYPE_BOOLEAN;
    }
    
    if (is_valid_number(value)) {
        return KIVA_TYPE_NUMBER;
    }
    
    return KIVA_TYPE_STRING;
}

// --- Fonctions de Gestion du Format ---

/**
 * Identifie la version du fichier sur le disque en vérifiant la signature magique.
 */
int kiva_detect_format(FILE* file) {
    fseek(file, 0, SEEK_END);
    long file_size = ftell(file);
    fseek(file, 0, SEEK_SET);
    
    // Fichier vide = Nouveau format par défaut (V2)
    if (file_size == 0) return FORMAT_V2; 
    
    char signature[4];
    if (fread(signature, 1, 4, file) == 4 && 
        strncmp(signature, MAGIC_SIGNATURE, 4) == 0) {
        fseek(file, 0, SEEK_SET);
        return FORMAT_V2;
    }
    
    // Si pas de signature et pas vide, on considère que c'est du V1 (Legacy)
    fseek(file, 0, SEEK_SET);
    return FORMAT_V1;
}

/**
 * Parcourt le fichier de données pour reconstruire l'index en mémoire (HashTable).
 * Gère la rétrocompatibilité V1 et les métadonnées V2 (Type + TTL).
 */
void kiva_load_index(KivaDB* db) {
    fseek(db->file, 0, SEEK_SET);
    int format = kiva_detect_format(db->file);
    
    // Si format V2, on valide et saute le header global (12 octets)
    if (format == FORMAT_V2) {
        KivaHeader header;
        if (fread(&header, sizeof(KivaHeader), 1, db->file) != 1) return; 
        if (header.format_version == FORMAT_V1) {
            fprintf(stderr, "[Warning] Format mismatch in header. Run 'compact' to synchronize.\n");
        }
    }
    
    uint32_t k_size, v_size;
    uint8_t type_raw;
    int64_t expires_at = 0;
    
    // Boucle de lecture séquentielle (Log-structured storage)
    while (fread(&k_size, sizeof(uint32_t), 1, db->file) == 1) {
        if (fread(&v_size, sizeof(uint32_t), 1, db->file) != 1) break;
        
        if (format == FORMAT_V2) {
            // Lecture métadonnées V2 : Type (1 octet) + Expiration (8 octets)
            if (fread(&type_raw, sizeof(uint8_t), 1, db->file) != 1) break;
            if (fread(&expires_at, sizeof(int64_t), 1, db->file) != 1) break;
        } else {
            // Fallback V1 : Tout est considéré String, pas d'expiration
            type_raw = KIVA_TYPE_STRING;
            expires_at = 0;
        }
        
        char* key = (char*)malloc(k_size + 1);
        if (!key) break;
        fread(key, 1, k_size, db->file);
        key[k_size] = '\0';
        
        if (v_size == 0) {
            // Tombstone : On retire la clé de l'index
            index_remove(db, key);
        } else {
            // Vérification de l'expiration (Lazy Loading)
            if (expires_at > 0 && expires_at < (int64_t)time(NULL)) {
                // Donnée expirée : On avance le pointeur de fichier sans indexer
                fseek(db->file, v_size, SEEK_CUR);
            } else {
                // Donnée valide : Enregistrement de l'offset physique de la valeur
                int64_t current_offset = ftell(db->file);
                int ttl_remaining = (expires_at > 0) ? (int)(expires_at - time(NULL)) : 0;
                
                index_set_ex(db, key, current_offset, v_size, (KivaType)type_raw, ttl_remaining);
                
                // Sauter la valeur pour lire l'entrée suivante
                fseek(db->file, v_size, SEEK_CUR);
            }
        }
        free(key);
    }
}