#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h> // Pour mesurer le temps
#include "../../include/kivadb.h"
#include "../core/kivadb_internal.h"
#include <direct.h> // Pour _mkdir sur Windows

void print_help() {
    printf("\n--- KivaDB Shell Help ---\n");
    printf("  set <key> <val>    : Create a NEW key-value pair (use \"\" for spaces)\n");
    printf("  update <key> <val> : Update an EXISTING key\n");
    printf("  get <key>          : Retrieve value of a key\n");
    printf("  typeof <key>       : Show the dynamic data type (string, number, boolean)\n");
    printf("  del <key>          : Remove a key from database\n");
    printf("  scan               : List all keys with their types and sizes\n");
    printf("  stats              : Show database health and file size\n");
    printf("  compact            : Reclaim disk space (defragmentation)\n");
    printf("  help or h          : Show this help menu\n");
    printf("  exit               : Close database and quit\n");
    printf("-------------------------\n");
}

int main(int argc, char* argv[]) {
    // Gestion des arguments de ligne de commande
    if (argc > 1) {
        if (
            strcmp(argv[1], "-version") == 0 
            || 
            strcmp(argv[1], "--version") == 0 
            || 
            strcmp(argv[1], "-v") == 0 
            || 
            strcmp(argv[1], "--v") == 0
        ) {
            printf("kivadb version %s\n", KIVADB_VERSION);
            return 0;
        }
        
        if (strcmp(argv[1], "--help") == 0 || strcmp(argv[1], "-h") == 0) {
            print_help();
            return 0;
        }
    }
    // 1. Création du dossier data s'il n'existe pas
    _mkdir("data");

    // 2. Nouveau chemin vers le fichier de stockage plus propre
    const char* db_path = "data/store.kiva";

    KivaDB* db = kiva_open(db_path);
    if (!db) {
        printf("Erreur : Impossible d'ouvrir ou créer %s\n", db_path);
        return 1;
    }

    char cmd[256], key[128], val[128];
    printf("KivaDB Shell v%s\n", KIVADB_VERSION);
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
            char *ptr = cmd + 4;
            char key[128], val[256];

            // On lit la clé
            if (sscanf(ptr, "%s", key) != 1) {
                printf("Usage: set <key> <value>\n");
                executed = 0;
            } else {
                ptr = strstr(ptr, key) + strlen(key);
                while (*ptr == ' ') ptr++; // On saute les espaces

                // Gestion des guillemets pour la valeur
                if (*ptr == '"') {
                    sscanf(ptr + 1, "%[^\"]", val);
                } else {
                    sscanf(ptr, "%s", val);
                }

                // Vérification si la clé existe déjà
                char* existing = kiva_get(db, key);
                if (existing) {
                    printf("Error: Key '%s' already exists. Use 'update'.\n", key);
                    free(existing);
                    executed = 0;
                } else {
                    kiva_set(db, key, val);
                    printf("OK (Created)");
                }
            }
        }
        // --- Logique pour UPDATE (Nouvelle commande) ---
        else if (strncmp(cmd, "update ", 7) == 0) {
            char *ptr = cmd + 7;
            sscanf(ptr, "%s", key);
            ptr = strstr(ptr, key) + strlen(key);
            while (*ptr == ' ') ptr++;
            
            if (*ptr == '"') sscanf(ptr + 1, "%[^\"]", val);
            else sscanf(ptr, "%s", val);

            char* existing = kiva_get(db, key);
            if (!existing) {
                printf("Error: Key '%s' not found.", key);
                executed = 0;
            } else {
                kiva_set(db, key, val);
                printf("OK (Updated)");
                free(existing);
            }
        }
        else if (strncmp(cmd, "get ", 4) == 0) {
            char *ptr = cmd + 4;
            // On utilise strtok pour découper la ligne par les espaces
            char *token = strtok(ptr, " "); 
            int found_args = 0;

            while (token != NULL) {
                // On ignore le mot-clé "and" s'il est présent pour permettre
                // la syntaxe : get key1 and key2
                if (strcmp(token, "and") != 0) {
                    found_args++;
                    char* res = kiva_get(db, token);
                    
                    // Formatage clair pour le multi-get : "clé: valeur"
                    printf("%s: %s\n", token, res ? res : "(nil)");
                    
                    if (res) {
                        free(res);
                    }
                }
                // On passe au mot suivant
                token = strtok(NULL, " ");
            }

            // Si l'utilisateur a juste tapé "get" sans rien derrière
            if (found_args == 0) {
                printf("Usage: get <key1> [and] <key2> ...\n");
                executed = 0;
            }
        }
        else if (strncmp(cmd, "typeof ", 7) == 0) {
            if (sscanf(cmd + 7, "%s", key) == 1) {
                printf("Type: %s", kiva_typeof(db, key));
            } else {
                printf("Usage: typeof <key>");
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
                // On récupère le statut renvoyé par le moteur
                KivaStatus status = kiva_delete(db, key);
                
                if (status == KIVA_OK) {
                    printf("OK (Deleted)");
                } else if (status == KIVA_ERR_NOT_FOUND) {
                    printf("Error: Key '%s' not found.", key);
                    executed = 0; // Optionnel : pour ne pas afficher le temps si c'est une erreur
                } else {
                    printf("Error: Could not delete key.");
                    executed = 0;
                }
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