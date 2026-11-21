#ifndef SKALD_VCS_H
#define SKALD_VCS_H

/* Generated from skald-vcs-ffi by cbindgen. */

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define SKALD_OK 0

#define SKALD_ERROR 1

#define SKALD_INVALID_ARGUMENT 2

#define SKALD_PANIC 3

#define SKALD_CONFLICT 4

struct SkaldRepo;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

uint32_t skald_abi_version(void);

const char *skald_last_error(void);

void skald_repo_free(struct SkaldRepo *handle);

void skald_string_free(char *value);

int32_t skald_init(const char *path, struct SkaldRepo **out);

int32_t skald_open(const char *path, struct SkaldRepo **out);

int32_t skald_clone(const char *path, const char *url, const char *token, struct SkaldRepo **out);

int32_t skald_stage(const struct SkaldRepo *handle, const char *path);

int32_t skald_unstage(const struct SkaldRepo *handle, const char *path);

int32_t skald_commit(const struct SkaldRepo *handle,
                     const char *author,
                     const char *message,
                     char **out);

int32_t skald_status(const struct SkaldRepo *handle, char **out_json);

int32_t skald_history(const struct SkaldRepo *handle,
                      const char *revision,
                      uint32_t limit,
                      char **out_json);

int32_t skald_branches(const struct SkaldRepo *handle, char **out_json);

int32_t skald_branch(const struct SkaldRepo *handle, const char *name, const char *from);

int32_t skald_branch_delete(const struct SkaldRepo *handle, const char *name, uint8_t force);

int32_t skald_checkout(const struct SkaldRepo *handle, const char *target, uint8_t detach);

int32_t skald_merge(const struct SkaldRepo *handle,
                    const char *target,
                    const char *author,
                    const char *message,
                    char **out_json);

int32_t skald_merge_abort(const struct SkaldRepo *handle);

int32_t skald_remote(const struct SkaldRepo *handle, const char *name, const char *url);

int32_t skald_fetch(const struct SkaldRepo *handle,
                    const char *remote,
                    const char *token,
                    char **out_json);

int32_t skald_push(const struct SkaldRepo *handle,
                   const char *remote,
                   const char *token,
                   const char *name,
                   uint8_t tags,
                   uint8_t delete_);

int32_t skald_pull(const struct SkaldRepo *handle,
                   const char *remote,
                   const char *token,
                   const char *branch,
                   const char *author,
                   char **out_json);

int32_t skald_verify(const struct SkaldRepo *handle);

int32_t skald_restore(const struct SkaldRepo *handle, const char *revision, const char *path);

int32_t skald_diff(const struct SkaldRepo *handle,
                   const char *from,
                   const char *to,
                   uint8_t staged,
                   char **out_json);

int32_t skald_tags(const struct SkaldRepo *handle, char **out_json);

int32_t skald_tag(const struct SkaldRepo *handle,
                  const char *name,
                  const char *from,
                  uint8_t delete_);

int32_t skald_remotes(const struct SkaldRepo *handle, char **out_json);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* SKALD_VCS_H */
