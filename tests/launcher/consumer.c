#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <readline/readline.h>
#include <readline/history.h>

static int provider(const char *name, void *address) {
    Dl_info info;
    if (!dladdr(address, &info) || !info.dli_fname) return 1;
    printf("%s=%s\n", name, info.dli_fname);
    return strstr(info.dli_fname, "liblineread.") == NULL;
}
int main(int argc, char **argv) {
    int bad = 0;
    bad |= provider("readline", (void *)readline);
    bad |= provider("add_history", (void *)add_history);
    bad |= provider("history_length", (void *)&history_length);
    if (argc != 3 || strcmp(argv[1], "space arg") || strcmp(argv[2], "")) return 2;
    add_history("launcher-test");
    if (history_length != 1) return 3;
    char *line = readline("");
    if (!line || strcmp(line, "hello")) return 4;
    free(line);
    return bad;
}
