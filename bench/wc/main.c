#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    char *line = NULL;
    size_t cap = 0;
    int64_t lines = 0, words = 0;
    while (getline(&line, &cap, stdin) != -1) {
        lines++;
        for (char *w = strtok(line, " \t\r\n\v\f"); w; w = strtok(NULL, " \t\r\n\v\f")) words++;
    }
    free(line);
    printf("%ld %ld\n", lines, words);
    return 0;
}
