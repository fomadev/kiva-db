#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <inttypes.h>
#include <ctype.h>
#include "../../include/kivadb.h"
#include "../core/kivadb_internal.h"

void print_help() {
    printf("\n--- KivaDB Shell Help ---\n");
    printf("  set [type] <key> <val> : Create NEW key-value pair(s)\n");
    printf("                           Types: string (needs \"\"), number, boolean\n");
    printf("                           Multiple: set k1 v1 and k2 v2\n");
    printf("  update <key> <val>     : Update EXISTING key(s)\n");
    printf("  get <key>              : Retrieve value of one or more keys\n");
    printf("  typeof <key>           : Show the dynamic data type\n");
    printf("  del <key>              : Remove one or more keys\n");
    printf("  scan                   : List all keys with their types and sizes\n");
    printf("  stats                  : Show database health and file size\n");
    printf("  compact                : Reclaim disk space (defragmentation)\n");
    printf("  help or h              : Show this help menu\n");
    printf("  exit                   : Close database and quit\n");
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

    MKDIR("data");
    const char* db_path = "data/store.kiva";

    KivaDB* db = kiva_open(db_path);
    if (!db) {
        printf("Erreur : Impossible d'ouvrir ou créer %s\n", db_path);
        return 1;
    }

    char cmd[256];
    printf("KivaDB Shell v%s\n", KIVADB_VERSION);
    printf("Type 'help' for commands\n");

    while (1) {
        printf("kiva> ");
        if (fgets(cmd, sizeof(cmd), stdin) == NULL) break;

        cmd[strcspn(cmd, "\n")] = 0;
        if (strlen(cmd) == 0) continue;

        clock_t start = clock();
        int executed = 1;

        // --- Commande SET (Version Strict v1.1.x) ---
        if (strncmp(cmd, "set ", 4) == 0) {
            char *ptr = cmd + 4;
            int created_count = 0;
            int found_args = 0;
            int expect_and = 0; // Verrou pour forcer le mot 'and'

            while (*ptr != '\0') {
                while (*ptr == ' ') ptr++;
                if (*ptr == '\0') break;

                // 1. Vérification du verrou 'and' pour le chaînage
                if (expect_and) {
                    if (strncmp(ptr, "and ", 4) == 0) {
                        ptr += 4;
                        while (*ptr == ' ') ptr++;
                        expect_and = 0;
                    } else {
                        printf("Error: Multiple assignments require 'and' keyword.\n");
                        executed = 0; break;
                    }
                }

                char key[128] = {0}, val[256] = {0};
                KivaType forced_type = KIVA_TYPE_UNKNOWN;

                // 2. Détection du type explicite
                if (strncmp(ptr, "string ", 7) == 0) { forced_type = KIVA_TYPE_STRING; ptr += 7; }
                else if (strncmp(ptr, "number ", 7) == 0) { forced_type = KIVA_TYPE_NUMBER; ptr += 7; }
                else if (strncmp(ptr, "boolean ", 8) == 0) { forced_type = KIVA_TYPE_BOOLEAN; ptr += 8; }
                while (*ptr == ' ') ptr++;

                // 3. Extraction de la CLÉ
                if (sscanf(ptr, "%127s", key) != 1) break;
                ptr = strstr(ptr, key) + strlen(key);
                while (*ptr == ' ') ptr++;

                // 4. Extraction de la VALEUR avec protection des types
                int has_quotes = (*ptr == '"');
                if (has_quotes) {
                    ptr++; int i = 0;
                    while (*ptr != '"' && *ptr != '\0' && i < 255) { val[i++] = *ptr++; }
                    if (*ptr == '"') ptr++;
                } else {
                    // Si pas de quotes, on extrait le mot pour l'analyser
                    if (sscanf(ptr, "%255s", val) == 1) {
                        // Protection : Si c'est du texte pur sans quotes -> Erreur
                        int is_num = 1;
                        for(int j=0; val[j]; j++) {
                            if(!isdigit(val[j]) && val[j]!='.' && val[j]!='-') is_num=0;
                        }

                        if (forced_type == KIVA_TYPE_STRING || (!is_num && forced_type == KIVA_TYPE_UNKNOWN)) {
                            if (strcmp(val, "true") != 0 && strcmp(val, "false") != 0) {
                                printf("Error: String values (like '%s') must be in \"quotes\".\n", val);
                                executed = 0; break;
                            }
                        }
                        ptr += strlen(val);
                    }
                }

                // 5. Validation finale et écriture
                if (strlen(key) > 0 && strlen(val) > 0) {
                    found_args++;
                    char* existing = kiva_get(db, key);
                    if (existing) {
                        printf("Error: Key '%s' already exists. Use 'update'.\n", key);
                        free(existing);
                        executed = 0; break;
                    } else {
                        kiva_set_with_type(db, key, val, forced_type);
                        printf("OK: %s set\n", key);
                        created_count++;
                        expect_and = 1; // Verrouille pour la prochaine paire
                    }
                } else {
                    printf("Error: Invalid syntax near key '%s'.\n", key);
                    executed = 0; break;
                }
                while (*ptr == ' ') ptr++;
            }
            if (found_args == 0 && executed) { printf("Usage: set [type] <key> <val>\n"); executed = 0; }
            else if (executed) printf("Summary: %d key(s) created.", created_count);
        }

        // --- Commande UPDATE ---
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

        // --- Commande GET ---
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

        // --- Commande DEL ---
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

        // --- Commande TYPEOF (Multi-clés) ---
        else if (strncmp(cmd, "typeof ", 7) == 0) {
            char *ptr = cmd + 7;
            char *token = strtok(ptr, " ");
            int found_args = 0;

            while (token != NULL) {
                // On ignore le mot-clé "and" pour permettre "typeof name and age"
                if (strcmp(token, "and") != 0) {
                    found_args++;
                    const char* type_str = kiva_typeof(db, token);
                    printf("%s: %s\n", token, type_str);
                }
                token = strtok(NULL, " ");
            }

            if (found_args == 0) {
                printf("Usage: typeof <key1> [and] <key2>...\n");
                executed = 0;
            }
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
            int64_t f_size = kiva_get_file_size(db_path);
            printf("\n--- Stats ---\nKeys: %d\nSize: %" PRId64 " bytes\n-------------", count, f_size);
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