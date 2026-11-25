// Diagnostic launcher for the real compiler and the executables it produces.
#include <errno.h>
#include <fcntl.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

extern char **environ;

static long long nanoseconds(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (long long)t.tv_sec * 1000000000 + t.tv_nsec;
}

int main(int argc, char **argv) {
    const char *base = strrchr(argv[0], '/');
    if (strcmp(base ? base + 1 : argv[0], "cc") == 0)
        argv[0] = "/usr/bin/cc";
    else if (argc > 1)
        argv++;
    else
        return 125;

    pid_t child;
    long long start = nanoseconds();
    int error = posix_spawn(&child, argv[0], NULL, NULL, argv, environ);
    if (error)
        return error == ENOENT ? 127 : 126;
    int status;
    while (waitpid(child, &status, 0) < 0)
        if (errno != EINTR)
            return 125;
    long long end = nanoseconds();
    int fd = open(getenv("CONFIGURE_CLOCK_LOG"), O_WRONLY | O_APPEND);
    if (fd < 0 || dprintf(fd, "%s %lld %lld\n", argv[0], start, end) < 0)
        return 125;
    close(fd);
    return WIFEXITED(status) ? WEXITSTATUS(status) : 128 + WTERMSIG(status);
}
