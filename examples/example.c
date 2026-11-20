
#include "loki_vcs.h"
#include <stdio.h>

int main(int argc, char **argv) {
    if (argc != 2 || lvcs_abi_version() != 1) {
        fprintf(stderr, "Usage: lvcs-example NEW_WORKING_DIRECTORY (ABI 1 required)\n");
        return 2;
    }
    struct LvcsRepo *repo = NULL;
    int32_t code = lvcs_init(argv[1], &repo);
    if (code != LVCS_OK) {
        fprintf(stderr, "%s\n", lvcs_last_error());
        return 1;
    }
    char *status = NULL;
    code = lvcs_status(repo, &status);
    if (code == LVCS_OK) puts(status);
    else fprintf(stderr, "%s\n", lvcs_last_error());
    lvcs_string_free(status);
    lvcs_repo_free(repo);
    return code == LVCS_OK ? 0 : 1;
}
