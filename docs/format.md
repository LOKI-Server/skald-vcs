
# Repository format v1

## Objects

An ID is a kind letter followed by 64 lowercase hexadecimal digits. The digest is BLAKE3 of the exact uncompressed payload. Kinds occupy separate namespaces, so identical bytes in different object types have different IDs.

| Prefix | Payload |
| --- | --- |
| c | The uncompressed bytes of one nonempty chunk. |
| f | A file manifest with size (unsigned byte count) and chunks (ordered array of descriptors, each with id and size). The sum of the descriptor sizes equals the file size. An empty file has zero descriptors. |
| t | A JSON object mapping each direct entry name to an object with id and executable. IDs reference file manifests or subtrees. A subtree entry has executable set to false. |
| r | A revision with parents, root, author, timestamp, and message. Parents is an ordered array with zero, one, or two revision IDs. Root is a tree ID. Timestamp is signed Unix seconds. |

Structured payloads are compact UTF-8 JSON without a BOM or final newline. Serialize fields in the order shown in the table. Directory entries are sorted lexicographically by UTF-8 bytes. Serialize numbers as minimal base-10 integers. Booleans are true or false. Use minimal JSON string escaping. Escape ASCII control codes with the standard short escape where defined, otherwise as lowercase \u00xx. Do not escape printable Unicode or forward slashes. The receiver reserializes the parsed object and requires byte equality. Unknown fields, duplicate fields, incorrect child types, or invalid lengths are rejected.

For example, an empty tree is exactly {} and an empty manifest is exactly {"size":0,"chunks":[]}. Binary chunks use their original bytes as the payload. No padding bytes or newline are added.

The local filesystem store maps an ID to objects/PPP/REST, with the first three characters of the ID in PPP. The first byte of the file is a codec byte. A zero is followed by the raw object payload. A one is followed by Zstandard-compressed chunk bytes. Readers bound both stored bytes and expanded bytes, with an 8 MiB object limit and a Zstandard window no larger than 8 MiB. The codec does not participate in the ID. A reader checks the hash and kind invariants before returning the payload.

A writer uses a streaming Gear hash to choose boundaries between the minimum and maximum chunk sizes. It uses a stricter mask before the target and a looser mask after it. Defaults are 64 KiB minimum, 256 KiB average target, 1 MiB maximum, and Zstandard level 3. Settings are not required to reconstruct a file. The first 32 KiB estimate compressibility before the writer attempts full compression of a chunk. Existing chunks are shared only inside a single repository.

Directories are Merkle trees. Equal trees and file manifests across revisions reuse existing objects. A file or directory component must be NFC UTF-8 and portable across Windows, Linux, and macOS. The format rejects traversal, control characters, reserved metadata names, reserved Windows names, alternate data streams, and names that collide under the format's uppercase comparison. The maximum path is 1,024 UTF-8 bytes and 64 components. A snapshot expands to at most 250,000 entries. Paths are always relative to the working copy.

## Working copy and recovery

The hidden .lvcs directory contains state.json, objects, a writer lock, and the disposable status cache. State includes the current branch or detached revision, index, local branch and tag refs, merge state, settings, and remote-tracking refs. Credentials are passed to the client separately. A branch can be unborn until its first commit. A tag points to an existing revision. The first parent of a merge is the previous tip of the current branch.

Object files are first written and synced to a temporary file, then installed with create-if-absent semantics. Metadata updates use atomic file replacement. Checkout first verifies that it can safely change the affected paths, then writes checkout.json before changing files. Reopening the copy resumes the checkout before any other operation. Keep external writers and editors stopped until recovery finishes. If recovery fails, preserve the working directory and .lvcs before examining the error or restoring backups.

Verification checks object hashes and file-manifest byte counts. GC marks from local branches, tags, the detached head, remote-tracking refs, the index, and pending merge parents, then removes expired unreachable objects. A removed branch or tag can cease to be recoverable after collection. Hosted stores also preserve references required by review records and allow an upload grace period.
