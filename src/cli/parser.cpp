#include "parser.hpp"
#include <iostream>
#include <cctype>

std::vector<std::string> CommandParser::tokenize(const std::string& input, std::vector<char>& delimiters) {
    std::vector<std::string> tokens;
    std::string current;
    char quote_char = 0;

    for (size_t i = 0; i < input.length(); ++i) {
        char c = input[i];

        if ((c == '"' || c == '\'' || c == '`') && quote_char == 0) {
            quote_char = c;
        } 
        else if (c == quote_char && quote_char != 0) {
            tokens.push_back(current);
            delimiters.push_back(quote_char);
            current.clear();
            quote_char = 0;
        } 
        else if (isspace(c) && quote_char == 0) {
            if (!current.empty()) {
                tokens.push_back(current);
                delimiters.push_back(0); 
                current.clear();
            }
        } 
        else {
            current += c;
        }
    }

    if (quote_char != 0) {
        std::cout << "Syntax Error: Unclosed quote detected (" << quote_char << ").\n";
        delimiters.clear();
        return {};
    }

    if (!current.empty()) {
        tokens.push_back(current);
        delimiters.push_back(0);
    }
    return tokens;
}