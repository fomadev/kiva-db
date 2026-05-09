/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#ifndef KIVADB_COMMANDS_HPP
#define KIVADB_COMMANDS_HPP

#include <vector>
#include <string>

extern "C" {
    #include "../../include/kivadb.h"
}

// Utilitaires partagés (définis dans handle_utils.cpp)
bool is_string_quote(char d);
bool is_backtick(char d);
bool is_bare(char d);
bool is_reserved_keyword(const std::string& key);

bool is_kiva_type(const std::string& t);

// Prototypes des commandes
void handle_set(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_get(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_update(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
// Mise à jour : handle_change ne prend plus 'delimiters'
void handle_change(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);
void handle_typeof(KivaDB** db, const std::vector<std::string>& tokens);
void handle_del(KivaDB** db, const std::vector<std::string>& tokens, const char* db_path);

void handle_stats(KivaDB** db);

void handle_has(KivaDB** db, const std::vector<std::string>& tokens);

void handle_print(KivaDB** db, const std::vector<std::string>& tokens, const std::vector<char>& delimiters);

#endif