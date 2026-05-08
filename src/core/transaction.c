/* * Copyright (c) 2026 Fordi / FomaDev. 
 * Licensed under FomaDev Public License.
 * See LICENSE file in the project root for full license information.
 */

#include "kivadb_internal.h"
#include <string.h>

/* Cross-platform file locking */
#ifdef _WIN32
    #include <io.h>
    #include <sys/locking.h>

    int kiva_lock_file(FILE* file) {
        int fd = fileno(file);
        if (fd == -1) return -1;
        _lseek(fd, 0L, SEEK_SET);
        // _LK_NBLCK : Verrouillage non-bloquant pour Windows
        return _locking(fd, _LK_NBLCK, 1L);
    }

    void kiva_unlock_file(FILE* file) {
        int fd = fileno(file);
        if (fd == -1) return;
        _lseek(fd, 0L, SEEK_SET);
        _locking(fd, _LK_UNLCK, 1L);
    }
#else
    #include <unistd.h>
    #include <fcntl.h>

    int kiva_lock_file(FILE* file) {
        int fd = fileno(file);
        if (fd == -1) return -1;

        struct flock fl;
        // Utilisation de memset pour éviter les warnings sur les initialiseurs
        memset(&fl, 0, sizeof(struct flock));
        
        fl.l_type = F_WRLCK;    // Verrouillage en écriture
        fl.l_whence = SEEK_SET;
        fl.l_start = 0;
        fl.l_len = 1;           // Verrouillage du premier octet (convention)

        // F_SETLK : Tentative de verrouillage non-bloquante
        return fcntl(fd, F_SETLK, &fl);
    }

    void kiva_unlock_file(FILE* file) {
        int fd = fileno(file);
        if (fd == -1) return;

        struct flock fl;
        memset(&fl, 0, sizeof(struct flock));
        
        fl.l_type = F_UNLCK;    // Déverrouillage
        fl.l_whence = SEEK_SET;
        fl.l_start = 0;
        fl.l_len = 1;
        
        fcntl(fd, F_SETLK, &fl);
    }
#endif