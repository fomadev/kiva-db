#ifndef KIVADB_INTERNAL_H
#define KIVADB_INTERNAL_H

#include <stdio.h>
#include <stdint.h>
#include "../../include/kivadb.h"

#define HASH_SIZE 1024

typedef struct {
    long offset;      
    uint32_t v_size;  
} KeyDirEntry;

typedef struct HashNode {
    char* key;
    KeyDirEntry entry;
    struct HashNode* next;
} HashNode;

struct KivaDB {
    FILE* file;
    char* path;
    HashNode* index[HASH_SIZE]; 
};

// --- Fonctions de index.c ---
unsigned long hash_function(const char* str);
void index_set(KivaDB* db, const char* key, long offset, uint32_t v_size);
void index_remove(KivaDB* db, const char* key);
void index_scan(KivaDB* db);

// --- Fonctions de transaction.c ---
int kiva_lock_file(FILE* file);
void kiva_unlock_file(FILE* file);


#endif