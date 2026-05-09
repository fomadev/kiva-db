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
 * Évalue une expression : gère les nombres, les clés, et l'arithmétique (+, -, *, /).
 * L'ordre de recherche (find_last_of) respecte la priorité des opérations.
 */
std::string evaluate_expression(KivaDB** db, std::string expr, PrintTokenType& out_type) {
    // 1. Nettoyage des espaces
    expr.erase(std::remove(expr.begin(), expr.end(), ' '), expr.end());
    if (expr.empty()) { out_type = P_ERROR; return ""; }

    // 2. Analyse des opérateurs (Priorité inverse pour la récursivité)
    // On cherche d'abord + et - car ils doivent être évalués en dernier (basse priorité)
    size_t op_pos = expr.find_last_of("+-");
    if (op_pos == std::string::npos) {
        // Sinon on cherche * et / (haute priorité)
        op_pos = expr.find_last_of("*/");
    }

    if (op_pos != std::string::npos && op_pos > 0 && op_pos < expr.length() - 1) {
        char op = expr[op_pos];
        std::string left_part = expr.substr(0, op_pos);
        std::string right_part = expr.substr(op_pos + 1);

        PrintTokenType t1, t2;
        std::string v1 = evaluate_expression(db, left_part, t1);
        std::string v2 = evaluate_expression(db, right_part, t2);

        if (t1 == P_NUMBER && t2 == P_NUMBER) {
            try {
                int val1 = std::stoi(v1);
                int val2 = std::stoi(v2);
                int res = 0;
                out_type = P_NUMBER;

                switch (op) {
                    case '+': res = val1 + val2; break;
                    case '-': res = val1 - val2; break;
                    case '*': res = val1 * val2; break;
                    case '/': 
                        if (val2 == 0) return "[DIV_BY_ZERO]";
                        res = val1 / val2; 
                        break;
                }
                return std::to_string(res);
            } catch (...) {
                out_type = P_ERROR; return "";
            }
        } else {
            return "[TYPE_ERROR]"; 
        }
    }

    // 3. Est-ce un nombre pur ?
    if (std::all_of(expr.begin(), expr.end(), ::isdigit)) {
        out_type = P_NUMBER;
        return expr;
    }

    // 4. Est-ce une clé dans la base ?
    char* db_val = kiva_get(*db, expr.c_str());
    if (db_val) {
        std::string res(db_val);
        const char* actual_type = kiva_typeof(*db, expr.c_str());
        free(db_val);
        
        out_type = (std::strcmp(actual_type, "number") == 0) ? P_NUMBER : P_STRING;
        return res;
    }

    out_type = P_ERROR;
    return "";
}

/**
 * handle_print (v2.1.5)
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

        // RÈGLE 1 : DÉTECTION OBLIGATOIRE DE SÉPARATEUR
        if (i > 1 && !is_quoted && token != "+" && token != ",") {
            if (tokens[i-1] != "+" && tokens[i-1] != ",") {
                std::cerr << "Error: SyntaxError: missing separator between tokens." << std::endl;
                return;
            }
        }

        if (!is_quoted && (token == "+" || token == ",")) continue;

        // RÈGLE 2 : ÉVALUATION ET INTERPOLATION
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

                if (sub_type == P_ERROR || sub_res == "[TYPE_ERROR]" || sub_res == "[DIV_BY_ZERO]") {
                    std::cerr << "Error: EvaluationError in '${" << sub_expr << "}': " 
                              << (sub_res == "[DIV_BY_ZERO]" ? "Division by zero" : "Invalid reference/type") << std::endl;
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

        if (current_type == P_ERROR || evaluated == "[TYPE_ERROR]" || evaluated == "[DIV_BY_ZERO]") {
            std::cerr << "Error: NameError/TypeError: " 
                      << (evaluated == "[DIV_BY_ZERO]" ? "Division by zero" : "Invalid operation") 
                      << " near '" << token << "'" << std::endl;
            return;
        }

        // RÈGLE 3 : CONCATÉNATION STRICTE
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