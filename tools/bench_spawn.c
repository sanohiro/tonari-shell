// Native controls for the corresponding workloads in bench_scripts.py.
#include <dirent.h>
#include <errno.h>
#include <spawn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

extern char **environ;

static int waited(pid_t pid) {
    int status;
    while (waitpid(pid, &status, 0) == -1) {
        if (errno != EINTR) return 0;
    }
    return WIFEXITED(status) && WEXITSTATUS(status) == 0;
}

// Read the same live directories. The two single-expansion controls count
// matching names only and deliberately exclude sorting and storing arguments.
static int directory(const char *name) {
    int sparse = strcmp(name, "glob-sparse.sh") == 0;
    int multi = strcmp(name, "glob-multi-star.sh") == 0;
    const char *path = sparse || multi ? "fixtures" :
                       strcmp(name, "glob-utf8-large.sh") == 0 ? "large" : "localized";
    int n = 0;
    char first[256] = "";
    for (int i = 0; i < (sparse || multi ? 30 : 1); ++i) {
        DIR *dir = opendir(path);
        if (!dir) return 1;
        struct dirent *entry;
        n = 0;
        first[0] = 0;
        while ((entry = readdir(dir))) {
            const char *s = entry->d_name;
            size_t len = strlen(s);
            if (len < 4 || strcmp(s + len - 4, ".txt") != 0) continue;
            if (sparse && strncmp(s, "item-000", 8) != 0) continue;
            if (multi && (len < 8 || strcmp(s + len - 6, "07.txt") != 0 ||
                          !strstr(s, "em") || strstr(s, "em") + 2 > s + len - 6)) continue;
            ++n;
            if ((sparse || multi) && (!first[0] || strcmp(s, first) < 0))
                snprintf(first, sizeof(first), "%s", s);
        }
        if (closedir(dir)) return 1;
    }
    if (sparse || multi) printf("%d %s/%s\n", n, path, first);
    else printf("%d\n", n);
    return 0;
}

static int pipeline(char *value, int merged, int builtin, int redirect_cat) {
    int fds[2];
    if (pipe(fds)) return 1;
    char *const print[] = {"/usr/bin/printf", "%s\n", value, NULL};
    char *const cat[] = {"/bin/cat", NULL};
    posix_spawn_file_actions_t actions;
    pid_t writer, reader;
    if (!builtin) {
        if (posix_spawn_file_actions_init(&actions) ||
            posix_spawn_file_actions_adddup2(&actions, fds[1], STDOUT_FILENO) ||
            (merged && posix_spawn_file_actions_adddup2(&actions, fds[1], STDERR_FILENO)) ||
            posix_spawn_file_actions_addclose(&actions, fds[0]) ||
            posix_spawn_file_actions_addclose(&actions, fds[1]) ||
            posix_spawn(&writer, print[0], &actions, NULL, print, environ)) return 1;
        posix_spawn_file_actions_destroy(&actions);
    }
    if (posix_spawn_file_actions_init(&actions) ||
        posix_spawn_file_actions_adddup2(&actions, fds[0], STDIN_FILENO) ||
        (redirect_cat && posix_spawn_file_actions_adddup2(&actions, STDOUT_FILENO, STDERR_FILENO)) ||
        posix_spawn_file_actions_addclose(&actions, fds[0]) ||
        posix_spawn_file_actions_addclose(&actions, fds[1]) ||
        posix_spawn(&reader, cat[0], &actions, NULL, cat, environ)) return 1;
    posix_spawn_file_actions_destroy(&actions);
    close(fds[0]);
    if (builtin && dprintf(fds[1], "%s\n", value) < 0) return 1;
    close(fds[1]);
    return (!builtin && !waited(writer)) || !waited(reader);
}

int main(int argc, char **argv) {
    if (argc < 4) return 1;
    if (strcmp(argv[3], "-c") == 0) {
        char gate;
        if (write(STDOUT_FILENO, "ready\n", 6) != 6 ||
            read(STDIN_FILENO, &gate, 1) != 1 || gate != '\n') return 1;
    }
    const char *path = strrchr(argv[argc-1], '/');
    const char *name = path ? path + 1 : argv[argc-1];
    if (strncmp(name, "glob-", 5) == 0) return directory(name);
    int env = strcmp(name, "external-env.sh") == 0;
    int redirect = strcmp(name, "redirect-stderr.sh") == 0;
    if (env) {
        for (int i = 0; i < 32; ++i) {
            char key[32], value[32];
            snprintf(key, sizeof(key), "BENCH_KEY_%d", i);
            snprintf(value, sizeof(value), "value%d", i);
            if (setenv(key, value, 1)) return 1;
        }
    }
    for (int i = 0; i < 16; ++i) {
        char value[16];
        snprintf(value, sizeof(value), "%d", i);
        if (strstr(name, "pipeline")) {
            if (pipeline(value, strstr(name, "merged") != NULL, strstr(name, "builtin") != NULL,
                         strstr(name, "redirect") != NULL)) return 1;
            continue;
        }
        char *command[] = {"/usr/bin/true", NULL, NULL, NULL};
        posix_spawn_file_actions_t actions;
        if (env) {
            command[0] = "/usr/bin/printenv";
            command[1] = "BENCH_KEY_31";
        } else if (redirect) {
            command[0] = "/usr/bin/printf";
            command[1] = "%s\n";
            command[2] = value;
            if (posix_spawn_file_actions_init(&actions) ||
                posix_spawn_file_actions_adddup2(&actions, STDOUT_FILENO, STDERR_FILENO)) return 1;
        }
        pid_t child;
        if (posix_spawn(&child, command[0], redirect ? &actions : NULL, NULL, command, environ)) return 1;
        if (redirect) posix_spawn_file_actions_destroy(&actions);
        if (!waited(child)) return 1;
    }
    if (!env && !redirect && !strstr(name, "pipeline")) puts("16");
    return 0;
}
