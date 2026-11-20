#ifndef LOKI_VCS_H
#define LOKI_VCS_H

/* Generated from loki-vcs-ffi by cbindgen. */

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>

#define LVCS_OK 0

#define LVCS_ERROR 1

#define LVCS_INVALID_ARGUMENT 2

#define LVCS_PANIC 3

#define LVCS_CONFLICT 4

struct LvcsRepo;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

uint32_t lvcs_abi_version(void);

const char *lvcs_last_error(void);

void lvcs_repo_free(struct LvcsRepo *handle);

void lvcs_string_free(char *value);

int32_t lvcs_init(const char *path, struct LvcsRepo **out);

int32_t lvcs_open(const char *path, struct LvcsRepo **out);

int32_t lvcs_clone(const char *path, const char *url, const char *token, struct LvcsRepo **out);

int32_t lvcs_stage(const struct LvcsRepo *handle, const char *path);

int32_t lvcs_unstage(const struct LvcsRepo *handle, const char *path);

int32_t lvcs_commit(const struct LvcsRepo *handle,
                    const char *author,
                    const char *message,
                    char **out);

int32_t lvcs_status(const struct LvcsRepo *handle, char **out_json);

int32_t lvcs_history(const struct LvcsRepo *handle,
                     const char *revision,
                     uint32_t limit,
                     char **out_json);

int32_t lvcs_branches(const struct LvcsRepo *handle, char **out_json);

int32_t lvcs_branch(const struct LvcsRepo *handle, const char *name, const char *from);

int32_t lvcs_branch_delete(const struct LvcsRepo *handle, const char *name, uint8_t force);

int32_t lvcs_checkout(const struct LvcsRepo *handle, const char *target, uint8_t detach);

int32_t lvcs_merge(const struct LvcsRepo *handle,
                   const char *target,
                   const char *author,
                   const char *message,
                   char **out_json);

int32_t lvcs_merge_abort(const struct LvcsRepo *handle);

int32_t lvcs_remote(const struct LvcsRepo *handle, const char *name, const char *url);

int32_t lvcs_fetch(const struct LvcsRepo *handle,
                   const char *remote,
                   const char *token,
                   char **out_json);

int32_t lvcs_push(const struct LvcsRepo *handle,
                  const char *remote,
                  const char *token,
                  const char *name,
                  uint8_t tags,
                  uint8_t delete_);

int32_t lvcs_pull(const struct LvcsRepo *handle,
                  const char *remote,
                  const char *token,
                  const char *branch,
                  const char *author,
                  char **out_json);

int32_t lvcs_verify(const struct LvcsRepo *handle);

int32_t lvcs_restore(const struct LvcsRepo *handle, const char *revision, const char *path);

int32_t lvcs_diff(const struct LvcsRepo *handle,
                  const char *from,
                  const char *to,
                  uint8_t staged,
                  char **out_json);

int32_t lvcs_tags(const struct LvcsRepo *handle, char **out_json);

int32_t lvcs_tag(const struct LvcsRepo *handle,
                 const char *name,
                 const char *from,
                 uint8_t delete_);

int32_t lvcs_remotes(const struct LvcsRepo *handle, char **out_json);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* LOKI_VCS_H */
