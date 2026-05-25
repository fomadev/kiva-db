/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 */

#include \"../commands.hpp\"
#include <iostream>
#include <string>
#include <vector>
#include <cstdlib>

extern "C" {
    bool index_bump(KivaDB* db, const char* key, int mode, int64_t ttl_sec);
}

void handle_bump(KivaDB** db, const std::vector<std::string>& tokens) {
    if (!db || !*db) return;

    size_t size = tokens.size();
    // Validations minimales de taille de tokens
    if (size < 4 || size > 5) {
        std::cerr << "Error: Usage: bump [type] <key> <add|set> <ttl_seconds>" << std::endl;
        return;
    }

    std::string type_target = "";
    std::string key = "";
    std::string op = "";
    std::string ttl_str = "";

    // Analyse de la structure selon la présence du paramètre optionnel [type]
    if (size == 5) {
        type_target = tokens[1];
        key = tokens[2];
        op = tokens[3];
        ttl_str = tokens[4];
    } else {
        key = tokens[1];
        op = tokens[2];
        ttl_str = tokens[3];
    }

    // 1. Validation du contrôle de type strict si spécifié
    if (!type_target.empty()) {
        const char* current_type = kiva_typeof(*db, key.c_str());
        if (type_target != current_type) {
            std::cerr << "Error: TypeError: Key '" << key << "' is of type '" 
                      << current_type << "', expected '" << type_target << "'" << std::endl;
            return;
        }
    }

    // 2. Validation de l'opération
    int mode = 0;
    if (op == "add") mode = 1;
    else if (op == "set") mode = 2;
    else {
        std::cerr << "Error: Invalid operation '" << op << "'. Use 'add' or 'set'." << std::endl;
        return;
    }

    // 3. Extraction du TTL
    int64_t ttl_val = std::atoll(ttl_str.c_str());
    if (ttl_val <= 0) {
        std::cerr << "Error: ValueError: TTL must be a positive integer." << std::endl;
        return;
    }

    // 4. Exécution du Bump dans l'index
    bool success = index_bump(*db, key.c_str(), mode, ttl_val);

    if (success) {
        std::cout << "OK: Key '" << key << "' TTL updated successfully (" << op << " " << ttl_val << "s)." << std::endl;
    } else {
        std::cerr << "Error: Key '" << key << "' not found or already expired." << std::endl;
    }
}