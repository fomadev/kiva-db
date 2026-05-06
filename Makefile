CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude

# --- Détection de l'OS et configuration des LDFLAGS ---
ifeq ($(OS), Windows_NT)
    LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static
    MKDIR_P = if not exist $(OBJ_DIR) mkdir $(OBJ_DIR)
    RM = del /Q /F
    RM_DIR = rmdir /S /Q
else
    LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static
    MKDIR_P = mkdir -p
    RM = rm -f
    RM_DIR = rm -rf
endif

# Configuration des dossiers
CORE_DIR = src/core
CLI_DIR = src/cli
HANDLE_DIR = src/cli/handle
OBJ_DIR = obj

# Sources Core C
C_CORE_SRCS = $(wildcard $(CORE_DIR)/*.c)

# Sources Core C++
CPP_CORE_SRCS = $(wildcard $(CORE_DIR)/*.cpp)

# Sources CLI (Fichiers à la racine de src/cli)
CPP_CLI_SRCS = $(wildcard $(CLI_DIR)/*.cpp)

# Sources HANDLE (Nouveaux fichiers modulaires dans src/cli/handle)
CPP_HANDLE_SRCS = $(wildcard $(HANDLE_DIR)/*.cpp)

# --- GÉNÉRATION DE LA LISTE DES OBJETS ---
C_CORE_OBJS = $(patsubst $(CORE_DIR)/%.c, $(OBJ_DIR)/%.o, $(C_CORE_SRCS))
CPP_CORE_OBJS = $(patsubst $(CORE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CORE_SRCS))
CPP_CLI_OBJS = $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CLI_SRCS))
CPP_HANDLE_OBJS = $(patsubst $(HANDLE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_HANDLE_SRCS))

OBJS = $(C_CORE_OBJS) $(CPP_CORE_OBJS) $(CPP_CLI_OBJS) $(CPP_HANDLE_OBJS)

TARGET = kivadb.exe

# --- RÈGLES PRINCIPALES ---

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- RÈGLES DE COMPILATION ---

# Compilation C (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@$(MKDIR_P)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation C++ (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.cpp
	@$(MKDIR_P)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (CLI principal)
$(OBJ_DIR)/%.o: $(CLI_DIR)/%.cpp
	@$(MKDIR_P)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (Handlers modulaires)
$(OBJ_DIR)/%.o: $(HANDLE_DIR)/%.cpp
	@$(MKDIR_P)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Nettoyage
clean:
	@echo "Cleaning objects and binary..."
	@$(RM_DIR) $(OBJ_DIR)
	@$(RM) $(TARGET)
	@$(RM) kivadb_win_x64.exe kivadb_mac_arm64 kivadb_linux_x64

.PHONY: all clean