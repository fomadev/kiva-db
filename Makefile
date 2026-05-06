# --- Configuration des Compilateurs ---
CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude

# --- Outils Standardisés (Compatibles w64devkit / Linux) ---
LDFLAGS = -lstdc++ -static-libgcc -static-libstdc++ -static
MKDIR_P = mkdir -p
RM = rm -f
RM_DIR = rm -rf

# --- Configuration des Dossiers ---
CORE_DIR = src/core
CLI_DIR = src/cli
HANDLE_DIR = src/cli/handle
OBJ_DIR = obj

# --- Détection des Sources ---
C_CORE_SRCS = $(wildcard $(CORE_DIR)/*.c)
CPP_CORE_SRCS = $(wildcard $(CORE_DIR)/*.cpp)
CPP_CLI_SRCS = $(wildcard $(CLI_DIR)/*.cpp)
CPP_HANDLE_SRCS = $(wildcard $(HANDLE_DIR)/*.cpp)

# --- Génération de la Liste des Objets ---
C_CORE_OBJS = $(patsubst $(CORE_DIR)/%.c, $(OBJ_DIR)/%.o, $(C_CORE_SRCS))
CPP_CORE_OBJS = $(patsubst $(CORE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CORE_SRCS))
CPP_CLI_OBJS = $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_CLI_SRCS))
CPP_HANDLE_OBJS = $(patsubst $(HANDLE_DIR)/%.cpp, $(OBJ_DIR)/%.o, $(CPP_HANDLE_SRCS))

OBJS = $(C_CORE_OBJS) $(CPP_CORE_OBJS) $(CPP_CLI_OBJS) $(CPP_HANDLE_OBJS)

TARGET = kivadb.exe

# --- Règles Principales ---

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- Gestion Automatique du Dossier d'Objets ---
# Cette règle crée le dossier obj s'il n'existe pas
$(OBJ_DIR):
	@$(MKDIR_P) $(OBJ_DIR)

# --- Règles de Compilation (avec | $(OBJ_DIR) pour l'ordre) ---

# Compilation C (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c | $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation C++ (Core)
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.cpp | $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (CLI principal)
$(OBJ_DIR)/%.o: $(CLI_DIR)/%.cpp | $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (Handlers modulaires dans src/cli/handle)
$(OBJ_DIR)/%.o: $(HANDLE_DIR)/%.cpp | $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# --- Nettoyage ---

clean:
	@echo "Cleaning KivaDB objects and binaries..."
	$(RM_DIR) $(OBJ_DIR)
	$(RM) $(TARGET)
	$(RM) kivadb_win_x64.exe kivadb_mac_arm64 kivadb_linux_x64

.PHONY: all clean