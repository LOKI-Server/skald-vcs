
# Loki VCS HTTPS protocol v1

Remote URLs refer to a repository root, such as https://host.example/lvcs/v1/repos/REPOSITORY_ID. Append the relative routes below to that URL. Use TLS, or loopback HTTP for development. The client will not follow redirects or use URLs containing passwords, query parameters, or fragments. Metadata and write-control requests use application/json. Raw object uploads and downloads use application/octet-stream with original object payload bytes. Files can be any supported size because binary content is transferred one chunk at a time.

Loki requires Authorization with the Bearer scheme. An alternative host may admit public readers or use its own bearer tokens. Authorization is checked for each request. Loki's dashboard can issue revocable account tokens with read or write scope. Paired API clients can exchange their management device grant for a short-lived, repository-scoped transfer token. Both token types follow current account and repository permissions. An author string in a revision is supplied by the client and is not an authenticated account assertion.

## Discovery and repository operations

| Method | Path | Result |
| --- | --- | --- |
| GET | /lvcs/v1 | Protocol version, object and negotiation limits, advisory lock policy, clone capability, and discovery URL. |
| GET | /lvcs/v1/repos | Visible repositories for the token. Entries include id, name, role, owner_id, and remote_url. |
| GET | repository root | Version, repository identity, default_branch, effective role, and lock policy. |
| GET | /refs | Refs object with version (1), default_branch, branches, and tags. Branches and tags map names to full revision IDs. An unborn default branch is absent from branches. |
| GET | /history | Recent graph history, as an array of [ID, revision object] pairs under revisions. Optional branch and limit query parameters select the branch and up to 100 entries. Read revision objects from /objects to traverse all history. |
| POST | /missing | Submit an object with ids, an array of up to 512 object IDs. Returns the same shape with only absent IDs. Requires write access on Loki. |
| GET | /objects/OBJECT_ID | Return a verified uncompressed object payload or 404. |
| PUT | /objects/OBJECT_ID | Accept an object payload and verify its ID and its schema. Repeating a verified upload is safe. Returns 204. |
| PUT | /refs/branches/NAME or /refs/tags/NAME | Submit expected and target IDs. For creation expected is null. For deletion target is null. Returns 204 after validation and atomic update. |
| GET | /locks | List active advisory locks. Each entry has owner and expires in Unix seconds. |
| PUT | /locks | Submit path and seconds, an integer from 60 through 86400. Pass null for seconds to release. Only the lock owner can renew or release it. |

Reference names use lowercase ASCII letters, digits, underscores, dots, dashes, and slash-separated components. Encode the entire name as one URL segment. For example, feature/terrain is feature%2Fterrain. A reference target is always a revision. A branch may only fast-forward, an existing tag cannot move, and the default branch cannot be deleted. Explicit deletion requires repository management access on Loki. A server may also require review on its default branch.

Implementations should return 400 for invalid object contents or invalid reference operations, 401 for missing or invalid credentials, 403 for insufficient privileges, 404 for an unknown or inaccessible repository or object, 409 for stale expected refs and competing locks, 413 for oversized input, 429 when transfer slots are busy, and 507 when quota or disk space is reached. Successful binary responses are raw. Loki's errors typically contain an error string. Clients should also handle empty or plain-text errors from a proxy.

## Synchronization

For a push, read /refs, build the revision closure, and batch object IDs through /missing. Send only absent payloads. The receiver verifies hashes and type invariants before making input available. Finally submit the observed reference value as expected and the proposed revision ID as target. The server validates the object graph and atomically compares and stores the new ref under the writer lock. A failed update leaves the old ref available. Upload dependencies may arrive in any order. An interrupted upload is resumed by repeating negotiation. Newly uploaded objects survive for at least 24 hours unless the repository is deleted. Object reads are scoped to the repository. Members with read permission can request an object with a known ID in that repository, including an unpublished upload.

For a fetch, read /refs and traverse objects by following revision parents and tree, file, and chunk references. Reuse verified objects already in the local store. Verify the reachable closure and then save the remote-tracking refs. A complete clone downloads the closures of all advertised refs and checks out the default branch. Offline work then uses repository files only. Partial and sparse modes are unsupported in v1.

Advisory locks coordinate binary changes. Pushes do not enforce them. A lock is independent from access to a branch and expires without modifying the work tree or reference graph.

The path namespace and the version field in refs identify v1. Clients should reject other versions. Compatible servers may add optional response fields. Changes to required fields, object payloads, or reference semantics require a new protocol version.
