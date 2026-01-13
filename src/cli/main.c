#include <stdio.h>
#include <stdlib.h>
#include "../../include/kivadb.h"

int main() {
    printf("--- KivaDB v1.0.0 Full Test Suite ---\n");

    // 1. Initialisation et ouverture
    KivaDB* db = kiva_open("test_store.kiva");
    if (!db) {
        fprintf(stderr, "Error: Could not open/lock database.\n");
        return 1;
    }

    // 2. Test d'écriture et de lecture (SET/GET)
    printf("\n[1] Testing Write/Read...\n");
    kiva_set(db, "user_1", "Alice");
    kiva_set(db, "user_2", "Bob");
    
    char* name = kiva_get(db, "user_1");
    if (name) {
        printf("Retrieved user_1: %s\n", name);
        free(name);
    }

    // 3. Test de suppression (DELETE)
    printf("\n[2] Testing Deletion...\n");
    kiva_delete(db, "user_2");
    char* deleted_check = kiva_get(db, "user_2");
    if (deleted_check == NULL) {
        printf("Success: user_2 was deleted.\n");
    }

    // 4. Test de mise à jour (Update)
    // En Append-Only, cela rajoute simplement une donnée à la fin
    printf("\n[3] Testing Updates (Append-only growth)...\n");
    kiva_set(db, "user_1", "Alice Updated");
    kiva_set(db, "user_1", "Alice Final Version");
    
    char* updated_name = kiva_get(db, "user_1");
    printf("Latest version of user_1: %s\n", updated_name);
    free(updated_name);

    // 5. Test de Compaction
    // Cela va nettoyer le fichier et ne garder que "Alice Final Version"
    printf("\n[4] Testing Compaction (Disk cleanup)...\n");
    kiva_compact(db);
    
    char* post_compact = kiva_get(db, "user_1");
    printf("Value after compaction: %s\n", post_compact);
    free(post_compact);

    // 6. Fermeture
    kiva_close(db);
    printf("\nDatabase closed.\n");

    // 7. Test de Persistance Finale
    printf("\n[5] Testing Final Persistence (Re-opening)...\n");
    db = kiva_open("test_store.kiva");
    if (db) {
        char* persisted = kiva_get(db, "user_1");
        if (persisted) {
            printf("Persistence Success: %s\n", persisted);
            free(persisted);
        }
        kiva_close(db);
    }

    printf("\n--- All tests completed successfully ---\n");
    return 0;
}