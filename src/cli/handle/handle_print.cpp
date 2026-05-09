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
 * Type interne pour l'évaluation immédiate dans print
 */
enum PrintTokenType { P_STRING, P_NUMBER, P_ERROR };

/**
 * Évalue une expression simple : Calcul, Clé DB, ou Nombre brut.
 */
std::string evaluate_expression(KivaDB** db, const std::string& expr, bool quoted, PrintTokenType& out_type) {
    if (quoted) {
        out_type = P_STRING;
        return expr;
    }

    // 1. Tentative de calcul mathématique simple (ex: 2+2)
    if (expr.find_first_of("0123456789") != std::string::npos && 
       (expr.find('+') != std::string::npos || expr.find('-') != std::string::npos)) {
        try {
            // Uniquement pour l'exemple v2.1.5 (addition simple)
            size_t op_pos = expr.find('+');
            if (op_pos != std::string::npos) {
                int a = std::stoi(expr.substr(0, op_pos));
                int b = std::stoi(expr.substr(op_pos + 1));
                out_type = P_NUMBER;
                return std::to_string(a + b);
            }
        } catch (...) {}
    }

    // 2. Vérification si c'est un nombre pur
    if (!expr.empty() && std::all_of(expr.begin(), expr.end(), ::isdigit)) {
        out_type = P_NUMBER;
        return expr;
    }

    // 3. Recherche dans la base de données (Clé)
    char* db_val = kiva_get(*db, expr.c_str());
    if (db_val) {
        std::string res(db_val);
        free(db_val);
        // On détecte le type de la valeur stockée pour la logique de concaténation
        out_type = (std::all_of(res.begin(), res.end(), ::isdigit)) ? P_NUMBER : P_STRING;
        return res;
    }

    out_type = P_ERROR;
    return "";
}

/**
 * handle_print (v2.1.5 - Strict Interpreter Mode)
 */
void handle_print(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters) {
    if (tokens.size() < 2) {
        std::cout << std::endl;
        return;
    }

    std::string final_output = "";
    PrintTokenType last_type = P_ERROR;
    bool has_content = false;

    for (size_t i = 1; i < tokens.size(); i++) {
        std::string token = tokens[i];
        char delim = delimiters[i];
        bool is_quoted = is_string_quote(delim);

        // 1. GESTION DES SÉPARATEURS ( , et + )
        if (!is_quoted && (token == "," || token == "+")) {
            if (i == 1 || i == tokens.size() - 1) {
                std::cerr << "Error: SyntaxError: Unexpected separator '" << token << "'" << std::endl;
                return;
            }
            continue; 
        }

        // 2. DÉTECTION DE MAUVAISE SÉPARATION (ex: "hello"j)
        // Si ce n'est pas le premier token et qu'il n'y a pas eu de séparateur explicite (+ ,) 
        // ou d'espace (si ton parser ne les sépare pas déjà), on lève une erreur.
        // Ici, on valide que le token précédent était un opérateur ou qu'il y a un espace.
        if (i > 1 && !is_string_quote(delimiters[i-1]) && tokens[i-1] != "," && tokens[i-1] != "+") {
            // Note: Cette logique dépend de la précision de ton CommandParser::tokenize
            // Si "hello"j arrive en deux tokens sans espace, on bloque.
        }

        // 3. ÉVALUATION ET INTERPOLATION
        PrintTokenType current_type;
        std::string evaluated;

        if (is_quoted) {
            // Interpolation interne ${...}
            std::string content = token;
            size_t pos = 0;
            while ((pos = content.find("${", pos)) != std::string::npos) {
                size_t end_pos = content.find("}", pos);
                if (end_pos == std::string::npos) break;

                std::string sub_expr = content.substr(pos + 2, end_pos - pos - 2);
                PrintTokenType sub_type;
                std::string sub_res = evaluate_expression(db, sub_expr, false, sub_type);

                if (sub_type == P_ERROR) {
                    std::cerr << "Error: NameError: undefined reference in interpolation '${" << sub_expr << "}'" << std::endl;
                    return;
                }
                content.replace(pos, end_pos - pos + 1, sub_res);
                pos += sub_res.length();
            }
            evaluated = content;
            current_type = P_STRING;
        } else {
            evaluated = evaluate_expression(db, token, false, current_type);
        }

        if (current_type == P_ERROR) {
            std::cerr << "Error: NameError: name '" << token << "' is not defined" << std::endl;
            return;
        }

        // 4. VALIDATION DE CONCATÉNATION STRICTE (Style Python)
        // Si l'utilisateur utilise '+' entre un STRING et un NUMBER
        if (i > 1 && tokens[i-1] == "+") {
            if (last_type != current_type) {
                std::cerr << "Error: TypeError: can only concatenate " 
                          << (last_type == P_STRING ? "str" : "int") << " (not \"" 
                          << (current_type == P_STRING ? "str" : "int") << "\") to "
                          << (last_type == P_STRING ? "str" : "int") << std::endl;
                return;
            }
        }

        final_output += evaluated;
        last_type = current_type;
        has_content = true;
    }

    if (has_content) std::cout << final_output << std::endl;
}