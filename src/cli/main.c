#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h> // Pour mesurer le temps
#include "../../include/kivadb.h"
#include "../core/kivadb_internal.h"

void print_help() {
    printf("\n--- KivaDB Shell Help ---\n");
    printf("  set <key> <val>  : Store or update a key\n");
    printf("  get <key>        : Retrieve value of a key\n");
    printf("  del <key>        : Remove a key from database\n");
    printf("  scan             : List all existing keys\n");
    printf("  stats            : Show database health and file size\n");
    printf("  compact          : Reclaim disk space (defragmentation)\n");
    printf("  help or h or \\h  : Show this help menu\n");
    printf("  exit             : Close database and quit\n");
    printf("-------------------------\n");
}

int main() {
    KivaDB* db = kiva_open("test_store.kiva");
    if (!db) {
        printf("Error: Could not open database.\n");
        return 1;
    }

    char cmd[256], key[128], val[128];
    printf("KivaDB Shell v1.0.0 (with timing)\n");
    printf("Type 'help' for commands\n");

    while (1) {
        printf("kiva> ");
        if (fgets(cmd, sizeof(cmd), stdin) == NULL) break;

        cmd[strcspn(cmd, "\n")] = 0;
        if (strlen(cmd) == 0) continue;

        // --- Début du chronomètre ---
        clock_t start = clock();
        int executed = 1;

        if (strncmp(cmd, "set ", 4) == 0) {
            if (sscanf(cmd + 4, "%s %[^\n]", key, val) == 2) {
                kiva_set(db, key, val);
                printf("OK");
            } else {
                printf("Usage: set <key> <value>");
            }
        } 
        else if (strncmp(cmd, "get ", 4) == 0) {
            char extra[128];
            // On essaie de lire la clé ET un éventuel argument supplémentaire
            int num_args = sscanf(cmd + 4, "%s %s", key, extra);

            if (num_args > 1) {
                printf("Error: 'get' command expects only 1 argument.\n");
                printf("Usage: get <key>\n");
                executed = 0;
            } else if (num_args == 1) {
                char* res = kiva_get(db, key);
                printf("%s\n", res ? res : "(nil)");
                if (res) free(res);
            } else {
                printf("Usage: get <key>\n");
                executed = 0;
            }
        } 
        else if (strncmp(cmd, "del ", 4) == 0) {
        char extra[128];
        int num_args = sscanf(cmd + 4, "%s %s", key, extra);
        
        if (num_args != 1) {
            printf("Error: 'del' command expects exactly 1 argument.\nUsage: del <key>\n");
            executed = 0;
        } else {
            kiva_delete(db, key);
            printf("OK\n");
        }
    }
        else if (strcmp(cmd, "compact") == 0) {
            kiva_compact(db);
            printf("Compaction done");
        }
        else if (strcmp(cmd, "help") == 0 || strcmp(cmd, "\\h") == 0 || strcmp(cmd, "h") == 0) {
            print_help();
            executed = 0;
        }
        else if (strcmp(cmd, "scan") == 0) {
            clock_t start = clock();
            index_scan(db);
            clock_t end = clock();
            double time_spent = (double)(end - start) / CLOCKS_PER_SEC;
            printf("Scan completed in %.6f sec\n", time_spent);
            executed = 0; // On évite le double affichage du temps dans le shell
        }
        else if (strcmp(cmd, "stats") == 0) {
            int count = 0;
            // On compte les clés dans l'index
            for (int i = 0; i < HASH_SIZE; i++) {
                HashNode* node = db->index[i];
                while (node) {
                    count++;
                    node = node->next;
                }
            }
            
            long f_size = kiva_get_file_size(db->path);
            
            printf("\n--- KivaDB Health Stats ---\n");
            printf("  Keys in RAM:       %d\n", count);
            printf("  File size on disk: %ld bytes\n", f_size);
            
            // Calcul de l'efficacité simple
            // Si la taille dépasse 100 octets par clé (arbitraire), on suggère de compacter
            if (count > 0 && f_size > (count * 150)) {
                printf("  Status:            Fragmentation high. Run 'compact'.\n");
            } else {
                printf("  Status:            Optimal\n");
            }
            printf("---------------------------\n");
            executed = 0;
        }
        else if (strcmp(cmd, "exit") == 0) {
            break;
        }
        else {
            printf("Unknown command.");
            executed = 0;
        }

        // --- Fin du chronomètre ---
        clock_t end = clock();
        if (executed) {
            double time_spent = (double)(end - start) / CLOCKS_PER_SEC;
            printf(" (%.6f sec)\n", time_spent);
        } else {
            printf("\n");
        }
    }

    kiva_close(db);
    printf("Bye!\n");
    return 0;
}