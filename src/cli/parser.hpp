/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#ifndef KIVADB_PARSER_HPP
#define KIVADB_PARSER_HPP

#include <string>
#include <vector>

struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input, std::vector<char>& delimiters);
};

#endif