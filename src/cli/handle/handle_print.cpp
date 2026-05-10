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
    // 1. Nettoyage des espaces pour l'analyse lexicale
    expr.erase(std::remove(expr.begin(), expr.end(), ' '), expr.end());
    if (expr.empty()) { out_type = P_ERROR; return ""; }

    // --- GESTION DES PARENTHÈSES (Priorité absolue) ---
    // On cherche la dernière parenthèse ouvrante pour évaluer le bloc le plus profond
    size_t last_open = expr.find_last_of('(');
    if (last_open != std::string::npos) {
        size_t close_pos = expr.find(')', last_open);
        if (close_pos != std::string::npos) {
            // Extraction du contenu : (sub_expr)
            std::string sub_expr = expr.substr(last_open + 1, close_pos - last_open - 1);
            PrintTokenType sub_type;
            std::string sub_res = evaluate_expression(db, sub_expr, sub_type);
            
            if (sub_type == P_ERROR || sub_res == "[DIV_BY_ZERO]" || sub_res == "[TYPE_ERROR]") {
                out_type = sub_type;
                return sub_res;
            }
            
            // Remplacement de "(...)" par son résultat et réévaluation
            expr.replace(last_open, close_pos - last_open + 1, sub_res);
            return evaluate_expression(db, expr, out_type);
        } else {
            // Parenthèse fermante manquante
            out_type = P_ERROR;
            return "";
        }
    }

    // --- ANALYSE DES OPÉRATEURS (Priorité : +,- puis *,/) ---
    // On cherche les opérateurs de basse priorité en premier pour la récursivité
    size_t op_pos = expr.find_last_of("+-");
    if (op_pos == std::string::npos) {
        op_pos = expr.find_last_of("*/");
    }

    // Si on trouve un opérateur (non situé aux extrémités)
    if (op_pos != std::string::npos && op_pos > 0 && op_pos < expr.length() - 1) {
        char op = expr[op_pos];
        std::string left_p = expr.substr(0, op_pos);
        std::string right_p = expr.substr(op_pos + 1);

        PrintTokenType t1, t2;
        std::string v1 = evaluate_expression(db, left_p, t1);
        std::string v2 = evaluate_expression(db, right_p, t2);

        if (t1 == P_NUMBER && t2 == P_NUMBER) {
            try {
                int a = std::stoi(v1);
                int b = std::stoi(v2);
                int res = 0;
                out_type = P_NUMBER;

                switch (op) {
                    case '+': res = a + b; break;
                    case '-': res = a - b; break;
                    case '*': res = a * b; break;
                    case '/': 
                        if (b == 0) return "[DIV_BY_ZERO]";
                        res = a / b; 
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

    // --- NOMBRES BRUTS ---
    if (std::all_of(expr.begin(), expr.end(), [](char c){ return std::isdigit(c) || c == '-'; }) && expr != "-") {
        out_type = P_NUMBER;
        return expr;
    }

    // --- CLÉS DE LA BASE (Vérification hybride) ---
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
 * handle_print (v2.1.6 - Support complet expressions parenthésées)
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

        // Séparateurs obligatoires
        if (i > 1 && !is_quoted && token != "+" && token != ",") {
            if (tokens[i-1] != "+" && tokens[i-1] != ",") {
                std::cerr << "Error: SyntaxError: missing separator between tokens." << std::endl;
                return;
            }
        }

        if (!is_quoted && (token == "+" || token == ",")) continue;

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
                              << (sub_res == "[DIV_BY_ZERO]" ? "Division by zero" : "Invalid syntax/type") << std::endl;
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
                      << (evaluated == "[DIV_BY_ZERO]" ? "Division by zero" : "Invalid reference or operation") 
                      << " near '" << token << "'" << std::endl;
            return;
        }

        // Concaténation stricte
        if (i > 1 && tokens[i-1] == "+") {
            if (last_type != current_type) {
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