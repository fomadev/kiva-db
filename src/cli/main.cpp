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

#ifndef MKDIR
    #ifdef _WIN32
        #include <direct.h>
        #define MKDIR(path) _mkdir(path)
    #else
        #include <sys/stat.h>
        #define MKDIR(path) mkdir(path, 0777)
    #endif
#endif

// Déclarée dans utils.cpp (ou autre fichier utilitaire)
void print_help();

int main(int argc, char* argv[]) {
    (void)argc; (void)argv;

    // Création du dossier de stockage si inexistant
    MKDIR("data");
    const char* db_path = "data/store.kiva";
    
    // Ouverture initiale de la base
    KivaDB* db = kiva_open(db_path);

    if (!db) {
        std::cerr << "Fatal Error: Could not open database." << std::endl;
        return 1;
    }

    std::cout << "KivaDB Shell v2.0.2.2\nType 'help' for commands\n";

    std::string line;
    while (true) {
        std::cout << "kiva> ";
        if (!std::getline(std::cin, line) || line == "exit") break;
        if (line.empty()) continue;

        std::vector<char> delimiters;
        auto tokens = CommandParser::tokenize(line, delimiters);
        if (tokens.empty()) continue;

        std::string cmd = tokens[0];
        clock_t start = clock();
        bool show_dur = true;

        /* 
         * Note technique : On passe l'adresse du pointeur (&db) aux fonctions de commande.
         * Cela permet à handle_del de réinitialiser la variable 'db' ici même.
         */
        
        if (cmd == "set") {
            handle_set(&db, tokens, delimiters);
        }
        else if (cmd == "update") {
            handle_update(&db, tokens, delimiters);
        }
        else if (cmd == "get") {
            handle_get(&db, tokens, delimiters);
        }
        else if (cmd == "change") {
            handle_change(&db, tokens, delimiters);
        }
        else if (cmd == "typeof") {
            handle_typeof(&db, tokens);
        }
        else if (cmd == "del") {
            handle_del(&db, tokens, db_path);
        }
        else if (cmd == "scan") {
            index_scan(db); 
            show_dur = false;
        }
        else if (cmd == "stats") {
            std::cout << "Keys: " << index_get_count(db) 
                      << " | File: " << kiva_get_file_size(db_path) << " bytes\n";
            show_dur = false;
        }
        else if (cmd == "compact") {
            kiva_compact(db); 
            std::cout << "Database compacted.\n";
        }
        else if (cmd == "help" || cmd == "h") {
            print_help(); 
            show_dur = false;
        }
        else {
            std::cout << "Unknown command.\n";
            show_dur = false;
        }

        // Affichage de la durée d'exécution (en microsecondes/secondes)
        if (show_dur) {
            double d = (double)(clock() - start) / CLOCKS_PER_SEC;
            std::cout << "(" << std::fixed << std::setprecision(6) << d << "s)\n";
        }
    }

    // Fermeture propre avant de quitter
    if (db) {
        kiva_close(db);
    }
    
    return 0;
}