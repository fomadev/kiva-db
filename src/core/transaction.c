#include "kivadb_internal.h"

/* Cross-platform file locking */
#ifdef _WIN32
    #include <io.h>
    #include <sys/locking.h>

    int kiva_lock_file(FILE* file) {
        int fd = fileno(file);
        _lseek(fd, 0L, SEEK_SET);
        // _LK_NBLCK : Non-blocking lock
        return _locking(fd, _LK_NBLCK, 1L);
    }

    void kiva_unlock_file(FILE* file) {
        int fd = fileno(file);
        _lseek(fd, 0L, SEEK_SET);
        _locking(fd, _LK_UNLCK, 1L);
    }
#else
    #include <unistd.h>
    #include <fcntl.h>

    int kiva_lock_file(FILE* file) {
        int fd = fileno(file);
        struct flock fl = {0};
        fl.l_type = F_WRLCK;    // Write lock
        fl.l_whence = SEEK_SET;
        fl.l_start = 0;
        fl.l_len = 1;           // Lock first byte (convention)
        // Use non-blocking lock
        return fcntl(fd, F_SETLK, &fl);
    }

    void kiva_unlock_file(FILE* file) {
        int fd = fileno(file);
        struct flock fl = {0};
        fl.l_type = F_UNLCK;    // Unlock
        fl.l_whence = SEEK_SET;
        fl.l_start = 0;
        fl.l_len = 1;
        fcntl(fd, F_SETLK, &fl);
    }
#endif