#include "kivadb_internal.h"
#include <string.h>
#include <stdlib.h>

// Algorithme de hachage
unsigned long hash_function(const char* str) {
    unsigned long hash = 5381;
    int c;
    while ((c = *str++)) {
        hash = ((hash << 5) + hash) + c;
    }
    return hash % HASH_SIZE;
}

// Supprimer une clé de l'index RAM
void index_remove(KivaDB* db, const char* key) {
    unsigned long h = hash_function(key);
    HashNode* node = db->index[h];
    HashNode* prev = NULL;

    while (node) {
        if (strcmp(node->key, key) == 0) {
            if (prev) prev->next = node->next;
            else db->index[h] = node->next;
            free(node->key);
            free(node);
            return;
        }
        prev = node;
        node = node->next;
    }
}

// Add or update a key in RAM index
void index_set(KivaDB* db, const char* key, int64_t offset, uint32_t v_size, KivaType type) {
    unsigned long h = hash_function(key);
    HashNode* node = db->index[h];

    while (node) {
        if (strcmp(node->key, key) == 0) {
            node->entry.offset = offset;
            node->entry.v_size = v_size;
            node->entry.type = type;
            return;
        }
        node = node->next;
    }

    HashNode* new_node = malloc(sizeof(HashNode));
    if (new_node) {
        new_node->key = strdup(key);
        new_node->entry.offset = offset;
        new_node->entry.v_size = v_size;
        new_node->entry.type = type; // On stocke le type pour les nouvelles clés
        new_node->next = db->index[h];
        db->index[h] = new_node;
    }
}

void index_scan(KivaDB* db) {
    printf("\n--- Current Keys in Database ---\n");
    int count = 0;
    for (int i = 0; i < HASH_SIZE; i++) {
        HashNode* node = db->index[i];
        while (node) {
            const char* type_str = "unknown";
            // Conversion du type enum en texte pour l'affichage
            if (node->entry.type == KIVA_TYPE_STRING) type_str = "string";
            else if (node->entry.type == KIVA_TYPE_NUMBER) type_str = "number";
            else if (node->entry.type == KIVA_TYPE_BOOLEAN) type_str = "boolean";

            printf("  -> %-15s | Type: %-8s | Size: %u bytes\n", 
                   node->key, type_str, node->entry.v_size);
            
            node = node->next;
            count++;
        }
    }
    printf("--------------------------------\n");
    printf("Total: %d keys found.\n", count);
}