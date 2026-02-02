#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h> 
#include "../../include/kivadb.h"
#include "../core/kivadb_internal.h"
#include <direct.h> // Pour _mkdir sur Windows

void print_help() {
    printf("\n--- KivaDB Shell Help ---\n");
    printf("  set <key> <val>    : Create NEW key-value pair(s) (use \"\" for spaces)\n");
    printf("  update <key> <val> : Update EXISTING key(s)\n");
    printf("  get <key>          : Retrieve value of one or more keys\n");
    printf("  typeof <key>       : Show the dynamic data type\n");
    printf("  del <key>          : Remove one or more keys\n");
    printf("  scan               : List all keys with their types and sizes\n");
    printf("  stats              : Show database health and file size\n");
    printf("  compact            : Reclaim disk space (defragmentation)\n");
    printf("  help or h          : Show this help menu\n");
    printf("  exit               : Close database and quit\n");
    printf("-------------------------\n");
}

int main(int argc, char* argv[]) {
    if (argc > 1) {
        if (strcmp(argv[1], "-version") == 0 || strcmp(argv[1], "--version") == 0 || 
            strcmp(argv[1], "-v") == 0 || strcmp(argv[1], "--v") == 0) {
            printf("kivadb version %s\n", KIVADB_VERSION);
            return 0;
        }
        if (strcmp(argv[1], "--help") == 0 || strcmp(argv[1], "-h") == 0) {
            print_help();
            return 0;
        }
    }

    _mkdir("data");
    const char* db_path = "data/store.kiva";

    KivaDB* db = kiva_open(db_path);
    if (!db) {
        printf("Erreur : Impossible d'ouvrir ou créer %s\n", db_path);
        return 1;
    }

    char cmd[256]; // 'key' et 'val' sont maintenant déclarées localement dans les boucles
    printf("KivaDB Shell v%s\n", KIVADB_VERSION);
    printf("Type 'help' for commands\n");

    while (1) {
        printf("kiva> ");
        if (fgets(cmd, sizeof(cmd), stdin) == NULL) break;

        cmd[strcspn(cmd, "\n")] = 0;
        if (strlen(cmd) == 0) continue;

        clock_t start = clock();
        int executed = 1;

        // --- Commande SET (Multi) ---
        if (strncmp(cmd, "set ", 4) == 0) {
            char *ptr = cmd + 4;
            int found_args = 0, created_count = 0;
            int executed = 1; // Utilisé pour valider l'affichage du chrono final

            // On pourrait imaginer ici un : clock_t start = clock(); 

            while (*ptr != '\0') {
                char key[128] = {0}, val[256] = {0};
                
                // Ignorer les espaces de début
                while (*ptr == ' ') ptr++;
                
                // Gérer le mot-clé optionnel "and" entre les paires
                if (strncmp(ptr, "and ", 4) == 0) { 
                    ptr += 4; 
                    while (*ptr == ' ') ptr++; 
                }
                
                if (*ptr == '\0') break;

                // 1. Extraction de la clé
                if (sscanf(ptr, "%127s", key) != 1) break;
                ptr = strstr(ptr, key) + strlen(key);
                while (*ptr == ' ') ptr++;

                // 2. Extraction de la valeur (sécurisée)
                if (*ptr == '"') {
                    ptr++; // Sauter le guillemet ouvrant
                    int i = 0;
                    // Sécurité : s'arrête si guillemet fermant, fin de chaîne ou buffer plein
                    while (*ptr != '"' && *ptr != '\0' && i < 255) { 
                        val[i++] = *ptr++; 
                    }
                    if (*ptr == '"') ptr++; // Sauter le guillemet fermant
                } else {
                    if (sscanf(ptr, "%255s", val) == 1) {
                        ptr += strlen(val);
                    }
                }

                // 3. Traitement en base de données
                if (strlen(key) > 0) {
                    found_args++;
                    char* existing = kiva_get(db, key);
                    
                    if (existing) {
                        printf("Error: Key '%s' already exists. Use 'update'.\n", key);
                        free(existing);
                    } else {
                        kiva_set(db, key, val);
                        printf("OK: %s set\n", key);
                        created_count++;
                    }
                }
                
                while (*ptr == ' ') ptr++;
            }

            // Résumé et gestion de la variable 'executed'
            if (found_args == 0) { 
                printf("Usage: set <key> <val> [and <key2> <val2>...]\n"); 
                executed = 0; 
            } else {
                printf("Summary: %d key(s) created.", created_count);
            }

            // Utilisation de 'executed' pour afficher le temps (résout le warning)
            if (executed) {
                // clock_t end = clock();
                // printf(" (%.6f sec)\n", (double)(end - start) / CLOCKS_PER_SEC);
                printf("\n"); // Simple retour à la ligne si vous gérez le temps ailleurs
            }
        }

        // --- Commande UPDATE (Multi) ---
        else if (strncmp(cmd, "update ", 7) == 0) {
            char *ptr = cmd + 7;
            int found_args = 0, updated_count = 0;

            while (*ptr != '\0') {
                char key[128] = {0}, val[256] = {0};
                while (*ptr == ' ') ptr++;
                if (strncmp(ptr, "and ", 4) == 0) { ptr += 4; while (*ptr == ' ') ptr++; }
                if (*ptr == '\0') break;

                if (sscanf(ptr, "%127s", key) != 1) break;
                ptr = strstr(ptr, key) + strlen(key);
                while (*ptr == ' ') ptr++;

                if (*ptr == '"') {
                    ptr++; int i = 0;
                    while (*ptr != '"' && *ptr != '\0' && i < 255) { val[i++] = *ptr++; }
                    if (*ptr == '"') ptr++;
                } else {
                    if (sscanf(ptr, "%255s", val) == 1) ptr += strlen(val);
                }

                if (strlen(key) > 0) {
                    found_args++;
                    char* existing = kiva_get(db, key);
                    if (!existing) {
                        printf("Error: Key '%s' not found.\n", key);
                    } else {
                        kiva_set(db, key, val);
                        printf("OK: %s updated\n", key);
                        updated_count++;
                        free(existing);
                    }
                }
                while (*ptr == ' ') ptr++;
            }
            if (found_args == 0) { printf("Usage: update <key> <val>...\n"); executed = 0; }
            else printf("Summary: %d key(s) updated.", updated_count);
        }

        // --- Commande GET (Multi) ---
        else if (strncmp(cmd, "get ", 4) == 0) {
            char *ptr = cmd + 4;
            char *token = strtok(ptr, " ");
            int found_args = 0;

            while (token != NULL) {
                if (strcmp(token, "and") != 0) {
                    found_args++;
                    char* res = kiva_get(db, token);
                    printf("%s: %s\n", token, res ? res : "(nil)");
                    if (res) free(res);
                }
                token = strtok(NULL, " ");
            }
            if (found_args == 0) { printf("Usage: get <key>...\n"); executed = 0; }
        }

        // --- Commande DEL (Multi) ---
        else if (strncmp(cmd, "del ", 4) == 0) {
            char *ptr = cmd + 4;
            char *token = strtok(ptr, " ");
            int found_args = 0, deleted_count = 0;

            while (token != NULL) {
                if (strcmp(token, "and") != 0) {
                    found_args++;
                    KivaStatus status = kiva_delete(db, token);
                    if (status == KIVA_OK) { printf("Deleted: %s\n", token); deleted_count++; }
                    else printf("Error: Key '%s' not found.\n", token);
                }
                token = strtok(NULL, " ");
            }
            if (found_args == 0) { printf("Usage: del <key>...\n"); executed = 0; }
            else printf("Summary: %d key(s) deleted.", deleted_count);
        }

        // --- Autres Commandes ---
        else if (strncmp(cmd, "typeof ", 7) == 0) {
            char k[128];
            if (sscanf(cmd + 7, "%s", k) == 1) printf("Type: %s", kiva_typeof(db, k));
            else { printf("Usage: typeof <key>"); executed = 0; }
        }
        else if (strcmp(cmd, "compact") == 0) {
            kiva_compact(db);
            printf("Compaction done");
        }
        else if (strcmp(cmd, "scan") == 0) {
            index_scan(db);
            executed = 0;
        }
        else if (strcmp(cmd, "stats") == 0) {
            int count = 0;
            for (int i = 0; i < HASH_SIZE; i++) {
                HashNode* node = db->index[i];
                while (node) { count++; node = node->next; }
            }
            long f_size = kiva_get_file_size(db->path);
            printf("\n--- Stats ---\nKeys: %d\nSize: %ld bytes\n-------------", count, f_size);
            executed = 0;
        }
        else if (strcmp(cmd, "help") == 0 || strcmp(cmd, "h") == 0) {
            print_help();
            executed = 0;
        }
        else if (strcmp(cmd, "exit") == 0) break;
        else { printf("Unknown command."); executed = 0; }

        clock_t end = clock();
        if (executed) printf(" (%.6f sec)\n", (double)(end - start) / CLOCKS_PER_SEC);
        else printf("\n");
    }

    kiva_close(db);
    printf("Bye!\n");
    return 0;
}