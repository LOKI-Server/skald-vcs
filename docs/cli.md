
# skald command reference

Pass --help to skald or any subcommand for its exact flags. Commands discover the working copy from the current directory. Add paths are relative to the current directory. Paths for unstage, restore, and locks use forward slashes and are relative to the repository root.

| Command | Behavior |
| --- | --- |
| init [directory] | Initialize an empty or existing working directory with an unborn main branch. |
| clone URL [new-directory] [--sparse] [--include PATH]... | Clone completely by default. Repeat `--include` for selected files or folders. Use `--sparse` alone to download metadata only. |
| status [--refresh] | List staged, unstaged, and unresolved changes. Refresh rehashes the working files and bypasses the stat cache. |
| add PATH... | Stage files, folders, or deletions. Add . stages the current directory. |
| unstage PATH... | Reset matching index paths to the current revision. |
| commit -m MESSAGE [--author NAME] | Create a revision from the index and advance the current branch. |
| branch [NAME] [--from REF] | List or create branches. Use branch --delete NAME and --force for explicitly deleting an unmerged branch. |
| switch NAME | Make a branch current and update its affected files. |
| checkout REF | Inspect a revision with a detached working copy. Create and switch to a branch before committing. |
| restore REF PATH... | Restore the named files from a revision into the working tree. Stage separately to include them in a commit. |
| log [REF] [-n COUNT] | Show revision IDs, authors, timestamps, messages, and ancestry from the selected tip. |
| diff [FROM] [TO] [--staged] | Compare the working tree or index, or compare two revisions. Reports binary changes by path. |
| merge REF [-m MESSAGE] [--author NAME] | Fast-forward or perform a three-way merge. See conflict handling below. |
| merge --abort | Revert an unfinished merge when the changes it must replace are clean. |
| tag [NAME] [--from REF] | List or create immutable tags. Use tag --delete NAME to remove a local tag. |
| remote list | List configured remotes. |
| remote set NAME URL | Add or change a remote. Tokens stay outside repository configuration. |
| remote remove NAME | Remove the remote and its cached refs. |
| fetch [REMOTE] | Fetch branches and tags to remote-tracking refs. |
| pull [REMOTE] [BRANCH] | Fetch and merge into the current branch. |
| push [REMOTE] [NAME] | Publish the current or named branch. Add --tag to push a tag. Add --delete to delete a remote ref, subject to permissions. |
| locks [REMOTE] | List unexpired advisory asset locks. |
| lock PATH [--remote NAME] [--seconds SECONDS] | Request or renew a lock. |
| unlock PATH [--remote NAME] | Release your lock. |
| config --min BYTES --average BYTES --max BYTES [--compression LEVEL] | Set chunk sizes and Zstandard level for subsequently staged files. |
| verify | Verify object hashes, manifest sizes, and availability of referenced objects. |
| gc [--grace-hours HOURS] | Collect unreachable local objects, including revisions whose last refs were deleted. Defaults to 24 hours. |

The default remote is origin. When a command creates a commit, use SKALD_AUTHOR or --author. Set SKALD_TOKEN in the process environment using a token from the hosting provider. Use separate shell sessions or a secret manager to supply credentials for different remotes. The client accepts HTTPS and loopback HTTP and rejects redirects. Logs and URLs contain no token.

To use the account token from Loki, use the remote URL shown in the repository dashboard.

~~~sh
skald clone https://loki.example.com/skald/v1/repos/REPOSITORY_ID game
cd game
skald branch artwork
skald switch artwork
skald add art
skald commit --author "Dev Name" -m "Update textures"
skald push
~~~

A reference can be a full revision ID, branch, tag, HEAD, or a remote-tracking ref such as remotes/origin/branches/main or remotes/origin/tags/v1. Fetch keeps tracking refs separate from local branches and tags. For example, create a local feature branch with skald branch feature --from remotes/origin/branches/feature.

## Sparse workspaces

~~~sh
skald clone https://loki.example.com/skald/v1/repos/REPOSITORY_ID game --sparse
cd game
skald workspace files
skald workspace add Assets/Characters ProjectSettings .skaldignore
skald workspace remove Assets/Characters/Archive --dry-run
skald workspace remove Assets/Characters/Archive
skald workspace prune
~~~

Use `workspace list` to see saved rules and `workspace files [REVISION]` to see tracked paths, their sizes in bytes, and whether each path is included. Rules match literal root-relative paths and their descendants using repository spelling. A more specific rule overrides its parent. Add or remove a directory to replace all rules beneath it. Use `workspace set PATH...` to replace the selection, `workspace set .` to include every file in the current snapshot, or `workspace set` for a metadata-only workspace. Set, add, and remove all accept `--dry-run`. Include `.skaldignore` if the repository uses it.

The selection follows branch switches. Removing paths from the workspace preserves their entries in commits and refuses to discard staged edits, working changes, or untracked files. `add .` stages only included paths. Current selected files are available offline. Historical content may require access to `origin`. The CLI downloads missing chunks during workspace expansion, checkout, restore, or merge using `SKALD_TOKEN`. Fetch updates the local metadata. A merge may fetch excluded files if both sides changed them, and an excluded conflict is added to the selection so it can be resolved. Historical diff lists changed paths even if uncached text is unavailable for a patch.

Removing a path leaves its chunks cached. Run `workspace prune` to reclaim chunks still reachable at `origin`, while retaining selected content, shared chunks, and chunks needed by unpushed revisions. A sparse workspace uses its original `origin` to recover missing chunks. Checkout and workspace changes verify the content they materialize. `verify` checks selected file content, local metadata, and cached objects while allowing uncached historical chunks.

## Working-tree and merge behavior

Use a root .skaldignore file with familiar glob syntax. Patterns can include comments, negations, subtree paths, or trailing slashes for directories. Already tracked files remain tracked when a matching rule is added.

Checkout rejects local changes or untracked paths it would overwrite, and leaves unrelated working changes in place. A merge with unresolvable paths retains the current version of those files, stages clean changes, and records the other parent. Open both versions with skald diff on the two branches, or use skald restore OTHER PATH to choose the other version. Stage each resolution explicitly and commit the merge. To choose a deletion, remove the working path and stage it. Text merging is bounded to 512 KiB per input and treats NUL-containing and non-UTF-8 data as binary. Histories with multiple best merge bases require reconciling the branches explicitly. Resolve incompatible case or file/directory names on a branch before merging.

The status cache uses observed file metadata. Use --refresh after tools that preserve file timestamps or when diagnosing filesystem inconsistencies. Staging and checkout safety checks read content independently of the cache. Separate processes serialize working copy operations through a lock. Keep external editors from writing while a checkout or recovery is in progress.
