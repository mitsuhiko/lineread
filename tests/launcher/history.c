#define _GNU_SOURCE
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <readline/history.h>
int main(void) {
    Dl_info info;
    if (!dladdr((void *)add_history, &info) || !info.dli_fname) return 2;
    printf("history-only add_history=%s\n", info.dli_fname);
    add_history("entry");
    return !strstr(info.dli_fname, "liblineread.") || history_length != 1;
}
