#include <setjmp.h>
#include <signal.h>
#include <stdint.h>
#include <string.h>

jmp_buf disk_full;

void sigbus_handler(int sig, siginfo_t *si, void *ctx) {
    if (si->si_code == BUS_ADRERR) {
        longjmp(disk_full, 1);
    }
}

int mmap_read(const void *mmap, void *data, size_t amnt) {
    struct sigaction sa, oldact = {0};
    sa.sa_sigaction = sigbus_handler;
    sa.sa_flags = SA_SIGINFO;
    sigfillset(&sa.sa_mask);
    sigaction(SIGBUS, &sa, &oldact);

    if (setjmp(disk_full) == 0) {
        memcpy(data, mmap, amnt);
    } else {
        return -1;
    }
    sigaction(SIGBUS, &oldact, NULL);
    return 0;
}

int mmap_write(void *mmap, const void *data, size_t amnt) {
    struct sigaction sa, oldact = {0};
    sa.sa_sigaction = sigbus_handler;
    sa.sa_flags = SA_SIGINFO;
    sigfillset(&sa.sa_mask);
    sigaction(SIGBUS, &sa, &oldact);

    if (setjmp(disk_full) == 0) {
        memcpy(mmap, data, amnt);
    } else {
        return -1;
    }
    sigaction(SIGBUS, &oldact, NULL);
    return 0;
}
