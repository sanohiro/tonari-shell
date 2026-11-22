// Native reference for bench_scripts.py's 16 sequential /usr/bin/true calls.
// The harness supplies the same environment, working directory and descriptors.
#define _POSIX_C_SOURCE 200809L
#include <errno.h>
#include <spawn.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

int main(int argc, char **argv) {
    if (argc < 4) return 1;
    if (strcmp(argv[3], "-c") == 0) {
        char gate;
        if (write(STDOUT_FILENO, "ready\n", 6) != 6 ||
            read(STDIN_FILENO, &gate, 1) != 1 || gate != '\n') return 1;
    }

    char *const command[] = {"/usr/bin/true", NULL};
    for (int i = 0; i < 16; ++i) {
        pid_t child;
        if (posix_spawn(&child, command[0], NULL, NULL, command, environ)) return 1;
        int status;
        pid_t waited;
        do {
            waited = waitpid(child, &status, 0);
        } while (waited < 0 && errno == EINTR);
        if (waited != child || !WIFEXITED(status) || WEXITSTATUS(status) != 0) return 1;
    }
    return write(STDOUT_FILENO, "16\n", 3) == 3 ? 0 : 1;
}
