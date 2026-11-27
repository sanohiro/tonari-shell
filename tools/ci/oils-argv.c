/* Timing control for Oils' Python 2 argv.py. Emit repr(sys.argv[1:]). */
#include <stdio.h>
#include <string.h>

static void argument(const char *value) {
    int quote = strchr(value, '\'') && !strchr(value, '"') ? '"' : '\'';
    putchar(quote);
    for (const unsigned char *p = (const unsigned char *)value; *p; p++) {
        if (*p == quote || *p == '\\') {
            putchar('\\');
            putchar(*p);
        } else if (*p == '\n') {
            fputs("\\n", stdout);
        } else if (*p == '\r') {
            fputs("\\r", stdout);
        } else if (*p == '\t') {
            fputs("\\t", stdout);
        } else if (*p < 32 || *p >= 127) {
            printf("\\x%02x", *p);
        } else {
            putchar(*p);
        }
    }
    putchar(quote);
}

int main(int argc, char **argv) {
    putchar('[');
    for (int i = 1; i < argc; i++) {
        if (i > 1) fputs(", ", stdout);
        argument(argv[i]);
    }
    puts("]");
    return fflush(stdout) == EOF || ferror(stdout);
}
