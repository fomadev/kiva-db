CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude
LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static

# Configuration des dossiers
CORE_DIR = src/core
CLI_DIR = src/cli
OBJ_DIR = obj

# Sources Core C
# Ajout de api.c, engine.c et format_v2.c
C_CORE_SRCS = $(CORE_DIR)/storage.c \
              $(CORE_DIR)/transaction.c \
              $(CORE_DIR)/api.c \
              $(CORE_DIR)/engine.c \
              $(CORE_DIR)/format_v2.c

# Sources Core C++
CPP_CORE_SRCS = $(CORE_DIR)/index.cpp

# Sources CLI (Détection automatique de main.cpp, commands.cpp, etc.)
CPP_CLI_SRCS = $(wildcard $(CLI_DIR)/*.cpp)

# --- GÉNÉRATION DE LA LISTE DES OBJETS ---

# Objets issus des fichiers C du Core
C_CORE_OBJS = $(patsubst $(CORE_DIR)/%.c, $(OBJ_DIR)/%.o, $(C_CORE_SRCS))

# Objets issus des fichiers C++ du Core
CPP_CORE_OBJS = $(patsubst $(CORE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CORE_SRCS))

# Objets issus des fichiers C++ du CLI
CPP_CLI_OBJS = $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CLI_SRCS))

# Liste finale de tous les objets à lier
OBJS = $(C_CORE_OBJS) $(CPP_CORE_OBJS) $(CPP_CLI_OBJS)

TARGET = kivadb.exe

# --- RÈGLES PRINCIPALES ---

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- RÈGLES DE COMPILATION ---

# Compilation des fichiers C (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@mkdir -p $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation des fichiers C++ (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation des fichiers C++ (CLI)
$(OBJ_DIR)/%.o: $(CLI_DIR)/%.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Nettoyage
clean:
	@echo "Cleaning objects and binary..."
	@rm -rf $(OBJ_DIR)
	@rm -f $(TARGET)

.PHONY: all clean