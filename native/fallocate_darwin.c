#define _FILE_OFFSET_BITS 64

#include <fcntl.h>
#include <stdint.h>

int native_fallocate(int fd, uint64_t len) {
    fstore_t fstore;
    fstore.fst_flags = F_ALLOCATECONTIG;
    fstore.fst_posmode = F_PEOFPOSMODE;
    fstore.fst_offset = 0;
    fstore.fst_length = len;
    fstore.fst_bytesalloc = 0;
    return fcntl(fd, F_PREALLOCATE, &fstore);
}
