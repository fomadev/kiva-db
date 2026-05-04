#ifndef KIVADB_PARSER_HPP
#define KIVADB_PARSER_HPP

#include <string>
#include <vector>

struct CommandParser {
    static std::vector<std::string> tokenize(const std::string& input, std::vector<char>& delimiters);
};

#endif