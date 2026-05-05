CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude

# --- Détection de l'OS et configuration des LDFLAGS ---
ifeq ($(OS), Windows_NT)
    # Configuration Windows (Linkage statique pour éviter les DLL manquantes)
    LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static
    MKDIR_P = mkdir -p
else
    UNAME_S := $(shell uname -s)
    ifeq ($(UNAME_S), Linux)
        # Configuration Linux (Linkage statique possible)
        LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static
    endif
    ifeq ($(UNAME_S), Darwin)
        # Configuration macOS (Linkage dynamique forcé par Apple)
        LDFLAGS = -lc++
    endif
    MKDIR_P = mkdir -p
endif

# Configuration des dossiers
CORE_DIR = src/core
CLI_DIR = src/cli
OBJ_DIR = obj

# Sources Core C
C_CORE_SRCS = $(CORE_DIR)/storage.c \
              $(CORE_DIR)/transaction.c \
              $(CORE_DIR)/api.c \
              $(CORE_DIR)/engine.c \
              $(CORE_DIR)/format_v2.c

# Sources Core C++
CPP_CORE_SRCS = $(CORE_DIR)/index.cpp

# Sources CLI (Détection automatique)
CPP_CLI_SRCS = $(wildcard $(CLI_DIR)/*.cpp)

# --- GÉNÉRATION DE LA LISTE DES OBJETS ---
C_CORE_OBJS = $(patsubst $(CORE_DIR)/%.c, $(OBJ_DIR)/%.o, $(C_CORE_SRCS))
CPP_CORE_OBJS = $(patsubst $(CORE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CORE_SRCS))
CPP_CLI_OBJS = $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CLI_SRCS))

OBJS = $(C_CORE_OBJS) $(CPP_CORE_OBJS) $(CPP_CLI_OBJS)

TARGET = kivadb.exe

# --- RÈGLES PRINCIPALES ---

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- RÈGLES DE COMPILATION ---

# Compilation C (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@$(MKDIR_P) $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation C++ (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.cpp
	@$(MKDIR_P) $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (CLI)
$(OBJ_DIR)/%.o: $(CLI_DIR)/%.cpp
	@$(MKDIR_P) $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Nettoyage
clean:
	@echo "Cleaning objects and binary..."
	@rm -rf $(OBJ_DIR)
	@rm -f $(TARGET)
	@rm -f kivadb_win_x64.exe kivadb_mac_arm64 kivadb_linux_x64

.PHONY: all clean