/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "../commands.hpp"
#include <iostream>
#include <cstring>
#include <algorithm>
#include <vector>
#include <string>

/**
 * handle_print (v2.1.5)
 * Affiche du texte brut ou des valeurs extraites de la base de données.
 * Supporte l'interpolation de variables via la syntaxe ${cle}.
 */
void handle_print(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    if (tokens.size() < 2) {
        std::cout << std::endl; // Print vide si aucun argument
        return;
    }

    for (size_t i = 1; i < tokens.size(); i++) {
        // Ignorer le mot-clé de liaison "and" pour permettre print "A" and "B"
        if (tokens[i] == "and" && !is_string_quote(delimiters[i])) {
            continue;
        }

        // CAS 1 : Accès direct (Bareword) -> print user_age
        if (!is_string_quote(delimiters[i])) {
            char* val = kiva_get(*db, tokens[i].c_str());
            if (val) {
                std::cout << val;
                free(val);
            } else {
                std::cout << "(nil)";
            }
        } 
        // CAS 2 : Chaîne de caractères avec interpolation -> print "Hello ${user}"
        else {
            std::string content = tokens[i];
            size_t pos = 0;

            // Analyse de l'interpolation ${...}
            while ((pos = content.find("${", pos)) != std::string::npos) {
                size_t end_pos = content.find("}", pos);
                if (end_pos == std::string::npos) break;

                // Extraction de la clé à l'intérieur des accolades
                std::string key_name = content.substr(pos + 2, end_pos - pos - 2);
                
                // Nettoyage des backticks éventuels ${`key`}
                key_name.erase(std::remove(key_name.begin(), key_name.end(), '`'), key_name.end());

                // Récupération de la valeur dans KivaDB
                char* db_val = kiva_get(*db, key_name.c_str());
                std::string replacement = (db_val) ? db_val : "[undefined]";
                
                // Remplacement dans la chaîne
                content.replace(pos, end_pos - pos + 1, replacement);
                
                // Libération de la mémoire si allouée
                if (db_val) free(db_val);

                // Avancer la position pour continuer la recherche
                pos += replacement.length();
            }
            std::cout << content;
        }

        // Ajouter un espace entre les arguments sauf si c'est le dernier
        if (i < tokens.size() - 1) {
            std::cout << " ";
        }
    }

    // Saut de ligne final
    std::cout << std::endl;
}