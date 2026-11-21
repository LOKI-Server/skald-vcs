
# Native SDK

The shared and static libraries expose the same C ABI. Use include/skald_vcs.h from the same release. Windows archives contain skald_vcs.dll, skald_vcs.dll.lib (MSVC import library), and skald_vcs.lib (static library). Linux archives contain libskald_vcs.so and libskald_vcs.a. macOS archives contain libskald_vcs.dylib and libskald_vcs.a.

The shared library needs to be discoverable by the platform loader. On Windows, put it beside the executable or configure an explicit application DLL directory. On Linux and macOS, use a platform-appropriate loader path or rpath. For static linking, use the native system libraries printed by this command for your target.

~~~sh
cargo rustc -p skald-vcs-ffi --release -- --print native-static-libs
~~~

## ABI and ownership

- skald_abi_version returns 1 as uint32_t. The header is authoritative for signatures. Status codes are int32_t and Boolean flags are uint8_t with values 0 and 1.
- Native calls are synchronous and can block on filesystem or a network request. Run them on a worker thread in a GUI. Serialize close against operations that use the same handle.
- All string arguments are valid, NUL-terminated UTF-8 while the call runs. A nullable string is explicitly marked optional in the table below. Path arguments accept UTF-8 names and use forward slashes for repository paths.
- On successful init, open, or clone the caller owns an opaque repository handle and releases it once with skald_repo_free. The caller owns returned strings and releases each once with skald_string_free, even on SKALD_CONFLICT. No caller allocation may be released with these functions. Null frees are supported.
- An output argument points to writable pointer-sized storage. The library clears it on entry. skald_last_error is a borrowed pointer valid on the calling thread until the next ABI operation on that thread. Copy the message before making another call and never free it.
- SKALD_OK is success. SKALD_ERROR reports an operation failure, SKALD_INVALID_ARGUMENT reports invalid pointers, flags, or UTF-8, SKALD_PANIC reports a caught unwind, and SKALD_CONFLICT reports a merge with unresolved paths that can be staged and committed. The caller must supply live handles and valid accessible pointers.

| Functions | Parameters and results |
| --- | --- |
| skald_init, skald_open | Directory path and output handle. |
| skald_clone | New directory, remote URL, optional token, output handle. |
| skald_stage, skald_unstage | Handle and root-relative path or dot. |
| skald_commit | Handle, author, message, output full revision ID. |
| skald_status | Handle, output JSON with branch, revision, staged, unstaged, and conflicts. |
| skald_history | Handle, optional revision, uint32 limit, output JSON array of pairs [revision ID, revision object]. |
| skald_branches | Handle, output JSON object mapping each branch to revision ID or null. |
| skald_branch, skald_branch_delete | Handle and branch name, plus optional source ref when creating or force flag when deleting. |
| skald_checkout | Handle, target, detach flag. Use 0 to switch branches and 1 to check out a revision. |
| skald_merge | Handle, ref, author, message, output JSON containing revision and conflicts. |
| skald_merge_abort | Handle. |
| skald_remote | Handle, remote name, URL. Pass null as URL to remove it. |
| skald_fetch | Handle, remote name, optional token, output protocol v1 refs JSON. |
| skald_push | Handle, remote name, optional token, optional ref name, tags flag, delete flag. |
| skald_pull | Handle, remote name, optional token, optional branch, author, output merge JSON. |
| skald_restore | Handle, revision, and root-relative file path. |
| skald_diff | Handle, optional from and to refs, staged flag, output JSON array with path, change, and optional unified patch. |
| skald_tags, skald_remotes | Handle, output JSON map of tag names to revision IDs or remote names to URLs. |
| skald_tag | Handle, name, optional source ref, and delete flag. |
| skald_verify | Handle. |

The ABI version changes for breaking changes. Releases within an ABI version keep exported signatures, status codes, ownership rules, and existing JSON fields. Consumers should accept new JSON fields. The on-disk format and network protocol each have their own version.

## Interoperability

Build either example from the release directory and link to the shared SDK.

~~~sh
cc -Iinclude examples/example.c -Llib -lskald_vcs -o skald-example
c++ -std=c++17 -Iinclude examples/example.cpp -Llib -lskald_vcs -o skald-example-cpp
~~~

For MSVC use cl /Iinclude examples\example.c lib\skald_vcs.dll.lib. Set the loader search path before running the examples. Each takes the path to a new or existing working directory, respectively.

For C# use DllImport with CallingConvention.Cdecl, IntPtr for handles and outputs, and UTF-8 marshaling for input strings. Read returned strings with Marshal.PtrToStringUTF8, then release them with skald_string_free. For Python ctypes, declare argtypes and restype and use c_void_p for owned strings, c_uint32 for sizes, and c_uint8 for flags. Use ctypes.string_at to copy data and then free it. For Java use the Foreign Function & Memory API or JNA, mapping native pointers to MemorySegment or Pointer. Keep allocations live for the duration of each call and close handles once.

Regenerate the header from the workspace root with cbindgen 0.28.0.

~~~sh
cbindgen --config cbindgen.toml --crate skald-vcs-ffi --output include/skald_vcs.h .
~~~
