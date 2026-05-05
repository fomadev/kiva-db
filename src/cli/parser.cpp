#include "parser.hpp"
#include <iostream>
#include <cctype>

/**
 * Tokenizer intelligent pour KivaDB.
 * Gère les guillemets (", '), les backticks (`) pour les clés,
 * et remplit le vecteur delimiters pour le contrôle de type strict.
 */
std::vector<std::string> CommandParser::tokenize(const std::string& input, std::vector<char>& delimiters) {
    std::vector<std::string> tokens;
    std::string current;
    char quote_char = 0;

    for (size_t i = 0; i < input.length(); ++i) {
        char c = input[i];

        // Détection de l'ouverture d'un délimiteur (", ', ou `)
        if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
            quote_char = c;
        } 
        // Détection de la fermeture du délimiteur correspondant
        else if (c == quote_char && quote_char != 0) {
            tokens.push_back(current);
            delimiters.push_back(quote_char); // On stocke ', ", ou `
            current.clear();
            quote_char = 0;
        } 
        // Gestion des espaces (séparateurs de tokens hors guillemets)
        else if (isspace(c) && quote_char == 0) {
            if (!current.empty()) {
                tokens.push_back(current);
                delimiters.push_back(0); // 0 indique un token "nu" (bare text)
                current.clear();
            }
        } 
        // Accumulation des caractères à l'intérieur d'un token
        else {
            current += c;
        }
    }

    // Gestion d'une erreur de syntaxe si un guillemet n'est pas fermé
    if (quote_char != 0) {
        std::cout << "Syntax Error: Unclosed quote detected (" << quote_char << ").\n";
        delimiters.clear();
        tokens.clear();
        return {};
    }

    // Ajout du dernier token si la ligne ne finit pas par un espace ou un guillemet
    if (!current.empty()) {
        tokens.push_back(current);
        delimiters.push_back(0);
    }

    return tokens;
}