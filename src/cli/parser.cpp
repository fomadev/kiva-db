/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "parser.hpp"
#include <iostream>
#include <cctype>

/**
 * Tokenizer intelligent pour KivaDB.
 * Gère les guillemets (", '), les backticks (`) pour les clés.
 * MISE À JOUR v2.1.6 : La virgule (,) est désormais un séparateur au même titre que l'espace,
 * permettant des syntaxes comme print "id",user sans erreur.
 */
std::vector<std::string> CommandParser::tokenize(const std::string& input, std::vector<char>& delimiters) {
    std::vector<std::string> tokens;
    std::string current;
    char quote_char = 0;

    for (size_t i = 0; i < input.length(); ++i) {
        char c = input[i];

        // 1. Détection de l'ouverture d'un délimiteur (", ', ou `)
        if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
            // Si on a du texte collé avant le guillemet (ex: print, "d"), on le valide d'abord
            if (!current.empty()) {
                tokens.push_back(current);
                delimiters.push_back(0);
                current.clear();
            }
            quote_char = c;
        } 
        // 2. Détection de la fermeture du délimiteur correspondant
        else if (c == quote_char && quote_char != 0) {
            tokens.push_back(current);
            delimiters.push_back(quote_char); // Stockage du type de guillemet
            current.clear();
            quote_char = 0;
        } 
        // 3. Gestion des séparateurs (Espace ou Virgule) hors guillemets
        else if ((isspace(c) || c == ',') && quote_char == 0) {
            if (!current.empty()) {
                tokens.push_back(current);
                delimiters.push_back(0); // 0 = Bare text / Key / Number
                current.clear();
            }
            
            // Si c'est une virgule, on l'ajoute comme un token propre pour handle_print
            if (c == ',') {
                tokens.push_back(",");
                delimiters.push_back(0);
            }
        } 
        // 4. Accumulation des caractères standards
        else {
            current += c;
        }
    }

    // Gestion d'erreur : Guillemet non fermé
    if (quote_char != 0) {
        std::cerr << "Syntax Error: Unclosed quote detected (" << quote_char << ").\n";
        delimiters.clear();
        tokens.clear();
        return {};
    }

    // Ajout du dernier token si nécessaire
    if (!current.empty()) {
        tokens.push_back(current);
        delimiters.push_back(0);
    }

    return tokens;
}