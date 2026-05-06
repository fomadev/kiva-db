#include <vector>
#include <string>
#include <utils.hpp>

/**
 * Vérifie si une chaîne est un mot-clé réservé.
 */
bool is_reserved_keyword(const std::string& key) {
    static const std::vector<std::string> keywords = {
        "set", "get", "update", "change", "del", "typeof", 
        "scan", "stats", "compact", "exit", "clear", "help",
        "string", "number", "boolean", "and", "ttl", "to", "all", "keys"
    };
    for (const auto& kw : keywords) {
        if (key == kw) return true;
    }
    return false;
}

/**
 * Utilitaires pour les délimiteurs.
 */
bool is_string_quote(char d) { return d == '"' || d == '\''; }
bool is_backtick(char d) { return d == '`'; }
bool is_bare(char d) { return d == 0; }