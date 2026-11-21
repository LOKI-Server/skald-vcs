
#include "skald_vcs.h"
#include <iostream>
#include <memory>

int main(int argc, char **argv) {
    if (argc != 2 || skald_abi_version() != 1) {
        std::cerr << "Usage: skald-example-cpp WORKING_DIRECTORY (ABI 1 required)\n";
        return 2;
    }
    SkaldRepo* raw = nullptr;
    if (skald_open(argv[1], &raw) != SKALD_OK) {
        std::cerr << skald_last_error() << '\n';
        return 1;
    }
    std::unique_ptr<SkaldRepo, decltype(&skald_repo_free)> repo(raw, skald_repo_free);
    char* history = nullptr;
    if (skald_history(repo.get(), nullptr, 20, &history) != SKALD_OK) {
        std::cerr << skald_last_error() << '\n';
        return 1;
    }
    std::unique_ptr<char, decltype(&skald_string_free)> result(history, skald_string_free);
    std::cout << result.get() << '\n';
}
