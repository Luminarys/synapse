#define _FILE_OFFSET_BITS 64

#include <fcntl.h>
#include <stdint.h>

int native_fallocate(int fd, uint64_t len) {
    fstore_t fstore;
    fstore.fst_flags = F_ALLOCATECONTIG;
    fstore.fst_posmode = F_PEOFPOSMODE;
    fstore.fst_offset = 0;
    fstore.fst_length = len;
    int res = fcntl(fd, F_PREALLOCATE, &fstore);
    if (res == -1) {
        // Due to fragmentation continuous allocation may not be possible
        fstore.fst_flags = F_ALLOCATEALL;
        res = fcntl(fd, F_PREALLOCATE, &fstore);
        if (res == -1) {
            return -1;
        }
    }
    return ftruncate(fd, len);
}
