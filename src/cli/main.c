#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h> // Pour mesurer le temps
#include "../../include/kivadb.h"
#include "../core/kivadb_internal.h"

void print_help() {
    printf("Commands: \n");
    printf("  set <key> <value>  : Store a pair\n");
    printf("  get <key>          : Retrieve a value\n");
    printf("  del <key>          : Remove a key\n");
    printf("  compact            : Clean the database file\n");
    printf("  exit               : Close KivaDB and leave\n");
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
            sscanf(cmd + 4, "%s", key);
            char* res = kiva_get(db, key);
            if (res) {
                printf("\"%s\"", res);
                free(res);
            } else {
                printf("(nil)");
            }
        } 
        else if (strncmp(cmd, "del ", 4) == 0) {
            sscanf(cmd + 4, "%s", key);
            kiva_delete(db, key);
            printf("OK");
        }
        else if (strcmp(cmd, "compact") == 0) {
            kiva_compact(db);
            printf("Compaction done");
        }
        else if (strcmp(cmd, "help") == 0) {
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