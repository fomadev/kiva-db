#include <iostream>
#include <string>
#include <vector>
#include <iomanip>
#include <ctime>

#include "parser.hpp"
#include "commands.hpp"

extern "C" {
    #include "../../include/kivadb.h"
    #include "../core/kivadb_internal.h"
}

// --- Gestion multiplateforme ---
#ifdef _WIN32
    #include <direct.h>
    #define CLEAR_COMMAND "cls"
    #ifndef MKDIR
        #define MKDIR(path) _mkdir(path)
    #endif
#else
    #include <sys/stat.h>
    #define CLEAR_COMMAND "clear"
    #ifndef MKDIR
        #define MKDIR(path) mkdir(path, 0777)
    #endif
#endif

/**
 * Affiche l'aide utilisateur pour les commandes du shell.
 */
void print_help();

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    // Création du dossier de données
    MKDIR("data");
    const char* db_path = "data/store.kiva";

    // Ouverture de la base de données
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open or initialize database at " << db_path << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v2.1.0\nType 'help' or 'h' for command list\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        // Analyse de la ligne
        std::vector<char> delimiters;
        auto tokens = CommandParser::tokenize(line, delimiters);
        
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_dur = true;

        // Routage des commandes
        if (cmd == "set") {
            handle_set(&db, tokens, delimiters);
        }
        else if (cmd == "update") {
            handle_update(&db, tokens, delimiters);
        }
        else if (cmd == "get") {
            handle_get(&db, tokens, delimiters);
        }
        else if (cmd == "del") {
            handle_del(&db, tokens, db_path);
        }
        else if (cmd == "clear") {
            system(CLEAR_COMMAND);
            show_dur = false;
        }
        else if (cmd == "change") {
            // Corrigé : Appel avec seulement 2 arguments
            handle_change(&db, tokens); 
        }
        else if (cmd == "typeof") {
            handle_typeof(&db, tokens);
        }
        else if (cmd == "scan") {
            index_scan(db); 
            show_dur = false;
        }
        else if (cmd == "stats") {
            std::cout << "Indexed Keys: " << index_get_count(db) 
                      << " | Storage: " << kiva_get_file_size(db_path) << " bytes\n";
            show_dur = false;
        }
        else if (cmd == "compact") {
            kiva_compact(db); 
            std::cout << "Database storage compacted successfully.\n";
        }
        else if (cmd == "help" || cmd == "h") {
            print_help(); 
            show_dur = false;
        }
        else {
            std::cout << "Unknown command: '" << cmd << "'. Try 'help'.\n";
            show_dur = false;
        }

        // Affichage du temps d'exécution
        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    if (db) kiva_close(db);
    std::cout << "Goodbye.\n";
    
    return 0;
}