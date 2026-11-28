/* Time the same precompiled, case-sensitive POSIX ERE as whole-language.py.
   Run with an available UTF-8 locale. Each pair swaps the measurement order. */
#include <locale.h>
#include <regex.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

static double milliseconds(void)
{
    struct timespec time;
    clock_gettime(CLOCK_MONOTONIC, &time);
    return time.tv_sec * 1000.0 + time.tv_nsec / 1000000.0;
}

int main(void)
{
    const size_t sizes[] = {0, 512, 1024, 2048, 8192};
    regex_t regex;
    if (setlocale(LC_ALL, "") == NULL || regcomp(&regex, "^f+hit$", REG_EXTENDED) != 0)
        return 1;

    puts("size\tcopy\ttrial\tms_per_1000");
    for (size_t i = 0; i < sizeof sizes / sizeof sizes[0]; ++i) {
        size_t size = sizes[i];
        char *text = malloc(size + 4);
        if (text == NULL)
            return 1;
        memset(text, 'f', size);
        memcpy(text + size, "hix", 4);
        for (int trial = 0; trial < 7; ++trial) {
            for (int pass = 0; pass < 2; ++pass) {
                int copy = (trial + pass) % 2;
                regmatch_t match;
                double start = milliseconds();
                for (int repetition = 0; repetition < 1000; ++repetition) {
                    char *subject = copy ? strdup(text) : text;
                    if (subject == NULL || regexec(&regex, subject, 1, &match, 0) != REG_NOMATCH)
                        return 1;
                    if (copy)
                        free(subject);
                }
                printf("%zu\t%d\t%d\t%.3f\n", size, copy, trial, milliseconds() - start);
            }
        }
        free(text);
    }
    regfree(&regex);
    return 0;
}
