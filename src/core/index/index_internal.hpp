/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#ifndef KIVADB_INDEX_INTERNAL_HPP
#define KIVADB_INDEX_INTERNAL_HPP

#include "../kivadb_internal.h" 
#include <unordered_map>
#include <string>

/**
 * Structure masquée encapsulant la HashTable STL.
 * Elle permet d'utiliser la puissance de la STL tout en restant opaque pour le C.
 */
struct KivaIndex {
    std::unordered_map<std::string, KeyDirEntry> map;
};

#endif // KIVADB_INDEX_INTERNAL_HPP