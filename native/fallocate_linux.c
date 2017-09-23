#define _GNU_SOURCE
#define _FILE_OFFSET_BITS 64

#include <fcntl.h>
#include <stdint.h>
#include <linux/falloc.h>

int native_fallocate(int fd, uint64_t len) {
    return fallocate(fd, 0, 0, (off_t)len);
}
