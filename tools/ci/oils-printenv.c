/* Timing control for the getenv loop in Oils' Python 2 printenv.py. */
#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv) {
    for (int i = 1; i < argc; i++) {
        const char *value = getenv(argv[i]);
        puts(value ? value : "None");
    }
    return ferror(stdout) != 0;
}
