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

enum PrintTokenType { P_STRING, P_NUMBER, P_ERROR };

/**
 * Évalue une expression : peut être un nombre, une clé, ou un calcul récursif (ex: a+b).
 * Version 2.1.5 - Support récursif et vérification de type.
 */
std::string evaluate_expression(KivaDB** db, std::string expr, PrintTokenType& out_type) {
    // 1. Nettoyage des espaces pour l'analyse lexicale
    expr.erase(std::remove(expr.begin(), expr.end(), ' '), expr.end());

    // 2. Tentative de calcul mathématique (Récursivité)
    size_t op_pos = expr.find_first_of("+-");
    if (op_pos != std::string::npos) {
        std::string left_part = expr.substr(0, op_pos);
        std::string right_part = expr.substr(op_pos + 1);

        PrintTokenType t1, t2;
        std::string v1 = evaluate_expression(db, left_part, t1);
        std::string v2 = evaluate_expression(db, right_part, t2);

        if (t1 == P_NUMBER && t2 == P_NUMBER) {
            try {
                int val1 = std::stoi(v1);
                int val2 = std::stoi(v2);
                int res = (expr[op_pos] == '+') ? val1 + val2 : val1 - val2;
                out_type = P_NUMBER;
                return std::to_string(res);
            } catch (...) {
                out_type = P_ERROR;
                return "";
            }
        } else {
            // Signalement d'un conflit de type dans une expression arithmétique
            return "[TYPE_ERROR]"; 
        }
    }

    // 3. Est-ce un nombre pur ?
    if (!expr.empty() && std::all_of(expr.begin(), expr.end(), ::isdigit)) {
        out_type = P_NUMBER;
        return expr;
    }

    // 4. Est-ce une clé dans la base ?
    char* db_val = kiva_get(*db, expr.c_str());
    if (db_val) {
        std::string res(db_val);
        // On récupère le type réel pour la logique de concaténation stricte
        const char* actual_type = kiva_typeof(*db, expr.c_str());
        free(db_val);
        
        out_type = (std::strcmp(actual_type, "number") == 0) ? P_NUMBER : P_STRING;
        return res;
    }

    out_type = P_ERROR;
    return "";
}

/**
 * handle_print (v2.1.5 - Strict Interpreter Mode)
 * Gère l'affichage avec interpolation, expressions récursives et séparateurs obligatoires.
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
        bool is_quoted = is_string_quote(delimiters[i]);

        // --- RÈGLE 1 : DÉTECTION OBLIGATOIRE DE SÉPARATEUR ---
        // Empêche les erreurs de type "hello"username
        if (i > 1 && !is_quoted && token != "+" && token != ",") {
            if (tokens[i-1] != "+" && tokens[i-1] != ",") {
                std::cerr << "Error: SyntaxError: missing separator between tokens." << std::endl;
                return;
            }
        }

        // On ignore les jetons séparateurs eux-mêmes pour le traitement de valeur
        if (!is_quoted && (token == "+" || token == ",")) continue;

        // --- RÈGLE 2 : ÉVALUATION ET INTERPOLATION ---
        PrintTokenType current_type;
        std::string evaluated;

        if (is_quoted) {
            std::string content = token;
            size_t pos = 0;
            while ((pos = content.find("${", pos)) != std::string::npos) {
                size_t end_pos = content.find("}", pos);
                if (end_pos == std::string::npos) break;

                std::string sub_expr = content.substr(pos + 2, end_pos - pos - 2);
                PrintTokenType sub_type;
                std::string sub_res = evaluate_expression(db, sub_expr, sub_type);

                if (sub_type == P_ERROR || sub_res == "[TYPE_ERROR]") {
                    std::cerr << "Error: NameError/TypeError in interpolation '${" << sub_expr << "}'" << std::endl;
                    return;
                }
                content.replace(pos, end_pos - pos + 1, sub_res);
                pos += sub_res.length();
            }
            evaluated = content;
            current_type = P_STRING;
        } else {
            evaluated = evaluate_expression(db, token, current_type);
        }

        // Gestion de l'erreur d'existence ou de type
        if (current_type == P_ERROR || evaluated == "[TYPE_ERROR]") {
            std::cerr << "Error: NameError/TypeError: invalid reference or operation near '" << token << "'" << std::endl;
            return;
        }

        // --- RÈGLE 3 : CONCATÉNATION STRICTE (Style Python) ---
        // Vérifie que l'opérateur '+' est utilisé entre types identiques
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