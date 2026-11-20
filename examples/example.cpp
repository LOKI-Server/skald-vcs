
#include "loki_vcs.h"
#include <iostream>
#include <memory>

int main(int argc, char **argv) {
    if (argc != 2 || lvcs_abi_version() != 1) {
        std::cerr << "Usage: lvcs-example-cpp WORKING_DIRECTORY (ABI 1 required)\n";
        return 2;
    }
    LvcsRepo* raw = nullptr;
    if (lvcs_open(argv[1], &raw) != LVCS_OK) {
        std::cerr << lvcs_last_error() << '\n';
        return 1;
    }
    std::unique_ptr<LvcsRepo, decltype(&lvcs_repo_free)> repo(raw, lvcs_repo_free);
    char* history = nullptr;
    if (lvcs_history(repo.get(), nullptr, 20, &history) != LVCS_OK) {
        std::cerr << lvcs_last_error() << '\n';
        return 1;
    }
    std::unique_ptr<char, decltype(&lvcs_string_free)> result(history, lvcs_string_free);
    std::cout << result.get() << '\n';
}
