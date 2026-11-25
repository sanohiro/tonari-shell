// Diagnostic launcher for the real compiler and the executables it produces.
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static long long nanoseconds(clockid_t clock) {
    struct timespec t;
    clock_gettime(clock, &t);
    return (long long)t.tv_sec * 1000000000 + t.tv_nsec;
}

int main(int argc, char **argv) {
    const char *base = strrchr(argv[0], '/');
    int compiler = strcmp(base ? base + 1 : argv[0], "cc") == 0;
    if (!compiler && argc == 2 && strcmp(argv[1], "--roundtrip") == 0) {
        // EPOCHREALTIME in the calling shell uses the same realtime clock.
        // The return interval includes this small write and process exit.
        long long entered = nanoseconds(CLOCK_REALTIME);
        long long leaving = nanoseconds(CLOCK_REALTIME);
        return dprintf(7, "%lld %lld\n", entered, leaving) < 0 ? 125 : 0;
    }
    if (compiler)
        argv[0] = "/usr/bin/cc";
    else if (argc > 1)
        argv++;
    else
        return 125;

    pid_t child;
    long long start = nanoseconds(CLOCK_MONOTONIC);
    int error = posix_spawn(&child, argv[0], NULL, NULL, argv, environ);
    if (error)
        return error == ENOENT ? 127 : 126;
    int status;
    struct rusage usage;
    while (wait4(child, &status, 0, &usage) < 0)
        if (errno != EINTR)
            return 125;
    long long end = nanoseconds(CLOCK_MONOTONIC);
    int fd = open(getenv("CONFIGURE_CLOCK_LOG"), O_WRONLY | O_APPEND);
    long long user_us = (long long)usage.ru_utime.tv_sec * 1000000 + usage.ru_utime.tv_usec;
    long long system_us = (long long)usage.ru_stime.tv_sec * 1000000 + usage.ru_stime.tv_usec;
    if (fd < 0 || dprintf(fd, "%s %lld %lld %lld %lld\n",
                          argv[0], start, end, user_us, system_us) < 0)
        return 125;
    close(fd);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
