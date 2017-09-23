#define _FILE_OFFSET_BITS 64

#include <fcntl.h>
#include <stdint.h>

int native_fallocate(int fd, uint64_t len) {
    return posix_fallocate(fd, 0, (off_t)len);
}
