#define _FILE_OFFSET_BITS 64

#include <fcntl.h>
#include <unistd.h>
#include <stdint.h>

int native_fallocate(int fd, uint64_t len) {
    // Use ftruncate here over posix_fallocate to prevent unnecessary IO on filesystems
    // like ZFS
    return ftruncate(fd, len);
}
