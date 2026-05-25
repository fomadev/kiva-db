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
INDEX_DIR = src/core/index
CLI_DIR = src/cli
HANDLE_DIR = src/cli/handle
OBJ_DIR = obj

# --- Détection des Sources ---
C_CORE_SRCS     = $(wildcard $(CORE_DIR)/*.c)
CPP_CORE_SRCS   = $(wildcard $(CORE_DIR)/*.cpp)
CPP_INDEX_SRCS  = $(wildcard $(INDEX_DIR)/*.cpp)
CPP_CLI_SRCS    = $(wildcard $(CLI_DIR)/*.cpp)
CPP_HANDLE_SRCS = $(wildcard $(HANDLE_DIR)/*.cpp)

# --- Génération de la Liste des Objets avec Préservation des Dossiers ---
C_CORE_OBJS     = $(patsubst $(CORE_DIR)/%.c, $(OBJ_DIR)/core/%.o, $(C_CORE_SRCS))
CPP_CORE_OBJS   = $(patsubst $(CORE_DIR)/%.cpp, $(OBJ_DIR)/core/%.o, $(CPP_CORE_SRCS))
CPP_INDEX_OBJS  = $(patsubst $(INDEX_DIR)/%.cpp, $(OBJ_DIR)/core/index/%.o, $(CPP_INDEX_SRCS))
CPP_CLI_OBJS    = $(patsubst $(CLI_DIR)/%.cpp, $(OBJ_DIR)/cli/%.o, $(CPP_CLI_SRCS))
CPP_HANDLE_OBJS = $(patsubst $(HANDLE_DIR)/%.cpp, $(OBJ_DIR)/cli/handle/%.o, $(CPP_HANDLE_SRCS))

OBJS = $(C_CORE_OBJS) $(CPP_CORE_OBJS) $(CPP_INDEX_OBJS) $(CPP_CLI_OBJS) $(CPP_HANDLE_OBJS)

TARGET = kivadb.exe

# --- Règles Principales ---

all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# --- Règles de Compilation Standardisées (Création dynamique des sous-dossiers obj) ---

# Compilation C (Core) -> obj/core/
$(OBJ_DIR)/core/%.o: $(CORE_DIR)/%.c
	@$(MKDIR_P) $(dir $@)
	$(CC) $(CFLAGS) -c $< -o $@

# Compilation C++ (Core) -> obj/core/
$(OBJ_DIR)/core/%.o: $(CORE_DIR)/%.cpp
	@$(MKDIR_P) $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (Index Modulaire) -> obj/core/index/
$(OBJ_DIR)/core/index/%.o: $(INDEX_DIR)/%.cpp
	@$(MKDIR_P) $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (CLI Principal) -> obj/cli/
$(OBJ_DIR)/cli/%.o: $(CLI_DIR)/%.cpp
	@$(MKDIR_P) $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Compilation C++ (Handlers) -> obj/cli/handle/
$(OBJ_DIR)/cli/handle/%.o: $(HANDLE_DIR)/%.cpp
	@$(MKDIR_P) $(dir $@)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# --- Nettoyage ---

clean:
	@echo "Cleaning KivaDB objects and binaries..."
	$(RM_DIR) $(OBJ_DIR)
	$(RM) $(TARGET)
	$(RM) kivadb_win_x64.exe kivadb_mac_arm64 kivadb_linux_x64

.PHONY: all clean