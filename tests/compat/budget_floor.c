// Match the kernel operations in specific audit scripts. Omit shell evaluation.
#include <errno.h>
#include <fcntl.h>
#include <stdio.h>
#include <spawn.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    int count = atoi(argv[2]);
    if (count < 1) return 2;
    if (puts("__READY__") < 0 || fflush(stdout) != 0 || getchar() == EOF) return 1;
    for (int i = 0; i < count; ++i) {
        if (!strcmp(argv[1], "groups")) {
            for (int j = 0; j < 3; ++j) {
                pid_t child = fork();
                if (child == 0) {
                    if (argc == 4) {
                        int fd = open("/dev/null", O_WRONLY);
                        if (fd < 0 || dup2(fd, 2) < 0) _exit(1);
                        if (fd != 2) close(fd);
                    }
                    _exit(0);
                }
                int status;
                if (child < 0 || waitpid(child, &status, 0) != child || status != 0) return 1;
            }
        } else if (!strcmp(argv[1], "file") && argc == 5) {
            int fd = open("file", O_CREAT | O_TRUNC | O_WRONLY, 0666);
            if (fd < 0 || write(fd, "data", 4) != 4 || close(fd)) return 1;
            for (int j = 0; j < 2; ++j) {
                const char *path = j ? "absent" : argv[4];
                struct stat info;
                if (!strcmp(argv[3], "-r")) (void)access(path, R_OK);
                else if (!strcmp(argv[3], "-w")) (void)access(path, W_OK);
                else (void)stat(path, &info);
            }
        } else if (!strcmp(argv[1], "wait") && argc == 4) {
            pid_t child;
            char *const args[] = {"sleep", argv[3], NULL};
            if (posix_spawnp(&child, "sleep", NULL, NULL, args, environ)) return 1;
            int status;
            if (waitpid(child, &status, 0) != child || status != 0) return 1;
        } else return 2;
    }
    return 0;
}
