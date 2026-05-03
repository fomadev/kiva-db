CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude
LDFLAGS = -lstdc++

# Configuration des dossiers
CORE_DIR = src/core
CLI_DIR = src/cli
OBJ_DIR = obj

# Sources
C_SRCS = $(CORE_DIR)/storage.c $(CORE_DIR)/transaction.c
CPP_SRCS = $(CORE_DIR)/index.cpp $(CLI_DIR)/main.cpp

# Objets
OBJS = $(OBJ_DIR)/storage.o $(OBJ_DIR)/transaction.o \
       $(OBJ_DIR)/index.o $(OBJ_DIR)/main.o

TARGET = kivadb.exe

# Règle principale
all: $(TARGET)

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# Règle générique pour les fichiers C
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@mkdir -p $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Règle spécifique pour index.cpp (situé dans CORE_DIR)
$(OBJ_DIR)/index.o: $(CORE_DIR)/index.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Règle spécifique pour main.cpp (situé dans CLI_DIR)
$(OBJ_DIR)/main.o: $(CLI_DIR)/main.cpp
	@mkdir -p $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Nettoyage
clean:
	rm -rf $(OBJ_DIR)
	rm -f $(TARGET)

.PHONY: all clean