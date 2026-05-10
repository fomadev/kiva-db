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
 * Évalue une expression : gère les parenthèses, les nombres, les clés, 
 * et l'arithmétique avec priorité des opérateurs.
 */
std::string evaluate_expression(KivaDB** db, std::string expr, PrintTokenType& out_type) {
    // 1. Nettoyage des espaces pour l'analyse lexicale interne
    expr.erase(std::remove(expr.begin(), expr.end(), ' '), expr.end());
    if (expr.empty()) { out_type = P_ERROR; return ""; }

    // --- GESTION DES PARENTHÈSES ---
    size_t last_open = expr.find_last_of('(');
    if (last_open != std::string::npos) {
        size_t close_pos = expr.find(')', last_open);
        if (close_pos != std::string::npos) {
            std::string sub_expr = expr.substr(last_open + 1, close_pos - last_open - 1);
            PrintTokenType sub_type;
            std::string sub_res = evaluate_expression(db, sub_expr, sub_type);
            
            if (sub_type == P_ERROR || sub_res == "[DIV_BY_ZERO]" || sub_res == "[TYPE_ERROR]") {
                out_type = sub_type;
                return sub_res;
            }
            
            expr.replace(last_open, close_pos - last_open + 1, sub_res);
            return evaluate_expression(db, expr, out_type);
        } else {
            out_type = P_ERROR; return "";
        }
    }

    // --- ANALYSE DES OPÉRATEURS (+, -, *, /) ---
    size_t op_pos = expr.find_last_of("+-");
    if (op_pos == std::string::npos) op_pos = expr.find_last_of("*/");

    if (op_pos != std::string::npos && op_pos > 0 && op_pos < expr.length() - 1) {
        char op = expr[op_pos];
        PrintTokenType t1, t2;
        std::string v1 = evaluate_expression(db, expr.substr(0, op_pos), t1);
        std::string v2 = evaluate_expression(db, expr.substr(op_pos + 1), t2);

        if (t1 == P_NUMBER && t2 == P_NUMBER) {
            try {
                int a = std::stoi(v1);
                int b = std::stoi(v2);
                int res = 0;
                out_type = P_NUMBER;
                if (op == '+') res = a + b;
                else if (op == '-') res = a - b;
                else if (op == '*') res = a * b;
                else if (op == '/') { if (b == 0) return "[DIV_BY_ZERO]"; res = a / b; }
                return std::to_string(res);
            } catch (...) { out_type = P_ERROR; return ""; }
        }
        return "[TYPE_ERROR]";
    }

    // --- NOMBRES ET CLÉS ---
    if (std::all_of(expr.begin(), expr.end(), [](char c){ return std::isdigit(c) || c == '-'; }) && expr != "-") {
        out_type = P_NUMBER; return expr;
    }

    char* db_val = kiva_get(*db, expr.c_str());
    if (db_val) {
        std::string res(db_val);
        const char* actual_type = kiva_typeof(*db, expr.c_str());
        free(db_val);
        out_type = (std::strcmp(actual_type, "number") == 0) ? P_NUMBER : P_STRING;
        return res;
    }

    out_type = P_ERROR; return "";
}

/**
 * Gère l'interpolation des chaînes ${...}
 */
std::string handle_interpolation(KivaDB** db, std::string content) {
    size_t pos = 0;
    while ((pos = content.find("${", pos)) != std::string::npos) {
        size_t end_pos = content.find("}", pos);
        if (end_pos == std::string::npos) break;

        std::string sub_expr = content.substr(pos + 2, end_pos - pos - 2);
        PrintTokenType sub_type;
        std::string sub_res = evaluate_expression(db, sub_expr, sub_type);

        if (sub_type == P_ERROR || sub_res == "[TYPE_ERROR]" || sub_res == "[DIV_BY_ZERO]") {
            return "[ERR]"; // Signal d'erreur court pour l'interpolation
        }
        content.replace(pos, end_pos - pos + 1, sub_res);
        pos += sub_res.length();
    }
    return content;
}

/**
 * handle_print (v2.1.6 - Final Robust Math & List support)
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

        // Nettoyage des virgules traînantes collées aux tokens (ex: age,)
        if (!is_quoted && token.length() > 1 && token.back() == ',') {
            token.pop_back();
        }

        // --- DÉTECTION DES OPÉRATEURS ET SÉPARATEURS ---
        bool is_op = (!is_quoted && (token == "+" || token == "-" || token == "*" || token == "/" || token == ","));
        
        // Règle de syntaxe : Si ce n'est pas un opérateur, le précédent devait en être un
        if (i > 1 && !is_quoted && !is_op) {
            std::string prev = tokens[i-1];
            bool prev_is_op = (prev == "+" || prev == "-" || prev == "*" || prev == "/" || prev == ",");
            if (!prev_is_op) {
                std::cerr << "Error: SyntaxError: missing separator between tokens." << std::endl;
                return;
            }
        }

        // Sauter le traitement si c'est un séparateur de liste (virgule)
        if (is_op && token == ",") continue;

        // Le '+' est traité comme concaténation SI il est entre deux jetons et non dans une expression mathématique
        // Mais pour simplifier, on laisse evaluate_expression gérer les calculs complexes.
        if (is_op && i > 1 && token != "+") continue; 

        PrintTokenType current_type;
        std::string evaluated;

        if (is_quoted) {
            evaluated = handle_interpolation(db, token);
            if (evaluated.find("[ERR]") != std::string::npos) {
                std::cerr << "Error: EvaluationError in string interpolation." << std::endl;
                return;
            }
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

        // Logique de concaténation stricte (+)
        if (i > 1 && tokens[i-1] == "+") {
            if (last_type != P_ERROR && last_type != current_type) {
                std::cerr << "Error: TypeError: cannot concatenate " 
                          << (last_type == P_STRING ? "str" : "int") << " and " 
                          << (current_type == P_STRING ? "str" : "int") << std::endl;
                return;
            }
        }

        final_output += evaluated;
        last_type = current_type;
        has_content = true;
    }

    if (has_content) std::cout << final_output << std::endl;
}