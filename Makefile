CC = gcc
CFLAGS = -Wall -Wextra -std=c11 -Iinclude
# Liste de tous les fichiers sources
SRC = src/core/storage.c src/core/index.c src/core/transaction.c src/cli/main.c
OBJ = $(SRC:.c=.o)
TARGET = kivadb_cli

all: $(TARGET)

$(TARGET): $(OBJ)
	$(CC) $(OBJ) -o $(TARGET)

clean:
	del /f /s *.o $(TARGET).exe