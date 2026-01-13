#ifndef KIVADB_INTERNAL_H
#define KIVADB_INTERNAL_H

#include <stdio.h>
#include "../../include/kivadb.h"

// Représente l'emplacement d'une valeur sur le disque
typedef struct {
    long offset;      // Position dans le fichier
    uint32_t v_size;  // Taille de la valeur
} KeyDirEntry;

// Une entrée simple pour notre Hash Map (chaînage en cas de collision)
typedef struct HashNode {
    char* key;
    KeyDirEntry entry;
    struct HashNode* next;
} HashNode;

#define HASH_SIZE 1024

struct KivaDB {
    FILE* file;
    char* path;
    HashNode* index[HASH_SIZE]; // Notre index en mémoire
};

// Fonction de hachage simple (djb2)
unsigned long hash_function(const char* str);

#endif