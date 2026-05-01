CC = gcc
CXX = g++
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
CXXFLAGS = -Wall -Wextra -std=c++17 -Iinclude
LDFLAGS = -lstdc++

# Sources
CORE_DIR = src/core
CLI_DIR = src/cli
OBJ_DIR = obj

# On sépare les objets C et C++
C_SRCS = $(CORE_DIR)/storage.c $(CORE_DIR)/transaction.c
# On prévoit de transformer index et main en C++
CPP_SRCS = $(CORE_DIR)/index.cpp $(CLI_DIR)/main.cpp

OBJS = $(OBJ_DIR)/storage.o $(OBJ_DIR)/transaction.o \
       $(OBJ_DIR)/index.o $(OBJ_DIR)/main.o

TARGET = kivadb.exe

$(TARGET): $(OBJS)
	$(CXX) $(OBJS) -o $(TARGET) $(LDFLAGS)

# Règle pour les fichiers C
$(OBJ_DIR)/%.o: $(CORE_DIR)/%.c
	@if not exist $(OBJ_DIR) mkdir $(OBJ_DIR)
	$(CC) $(CFLAGS) -c $< -o $@

# Règle pour les fichiers C++ (Core)
$(OBJ_DIR)/index.o: $(CORE_DIR)/index.cpp
	@if not exist $(OBJ_DIR) mkdir $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

# Règle pour le CLI C++
$(OBJ_DIR)/main.o: $(CLI_DIR)/main.cpp
	@if not exist $(OBJ_DIR) mkdir $(OBJ_DIR)
	$(CXX) $(CXXFLAGS) -c $< -o $@

clean:
	@if exist $(OBJ_DIR) rmdir /s /q $(OBJ_DIR)
	@if exist $(TARGET) del $(TARGET)