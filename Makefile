CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude
LDFLAGS = -lstdc++

# Configuration des dossiers
CORE_DIR = src/core
CLI_DIR = src/cli
OBJ_DIR = obj

# Sources Core (C et C++)
C_CORE_SRCS = $(CORE_DIR)/storage.c $(CORE_DIR)/transaction.c
CPP_CORE_SRCS = $(CORE_DIR)/index.cpp

# Sources CLI (Tous les fichiers .cpp du dossier cli)
# Cette commande détecte automatiquement : main.cpp, parser.cpp, commands.cpp, utils.cpp
CPP_CLI_SRCS = $(wildcard $(CLI_DIR)/*.cpp)

# Objets
# On transforme les chemins des sources en chemins d'objets dans le dossier obj/
OBJS = $(OBJ_DIR)/storage.o \
       $(OBJ_DIR)/transaction.o \
       $(OBJ_DIR)/index.o \
       $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CLI_SRCS))

TARGET = kivadb.exe

# Règle principale
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- RÈGLES DE COMPILATION ---

# Compilation des fichiers C (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@mkdir -p $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation des fichiers C++ (Core - index.cpp)
$(OBJ_DIR)/index.o: $(CORE_DIR)/index.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation des fichiers C++ (CLI - main, parser, commands, utils)
# Cette règle générique gère tous les nouveaux fichiers du dossier cli/
$(OBJ_DIR)/%.o: $(CLI_DIR)/%.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Nettoyage
clean:
	@echo "Cleaning objects and binary..."
	@rm -rf $(OBJ_DIR)
	@rm -f $(TARGET)

.PHONY: all clean