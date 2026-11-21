
#include "skald_vcs.h"
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc != 2 || skald_abi_version() != 1) {
        fprintf(stderr, "Usage: skald-example NEW_WORKING_DIRECTORY (ABI 1 required)\n");
        return 2;
    }
    struct SkaldRepo *repo = NULL;
    int32_t code = skald_init(argv[1], &repo);
    if (code != SKALD_OK) {
        fprintf(stderr, "%s\n", skald_last_error());
        return 1;
    }
    char *status = NULL;
    code = skald_status(repo, &status);
    if (code == SKALD_OK) puts(status);
    else fprintf(stderr, "%s\n", skald_last_error());
    skald_string_free(status);
    skald_repo_free(repo);
    return code == SKALD_OK ? 0 : 1;
}
