#ifndef KIVADB_COMMANDS_HPP
#define KIVADB_COMMANDS_HPP

#include <vector>
#include <string>
extern "C" {
    #include "../../include/kivadb.h"
}

void handle_set(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_get(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_update(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_change(KivaDB* db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_typeof(KivaDB* db, const std::vector<std::string>& tokens);
void handle_del(KivaDB* db, const std::vector<std::string>& tokens, const char* db_path);

#endif