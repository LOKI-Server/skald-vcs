
# Skald 0.1

Named for the Old Norse court poets, Skald is an offline-first version control system written in Rust. It stores revisions and directory snapshots by content, uses content-defined BLAKE3 chunks for all files, and compresses useful chunks with Zstandard. Binary assets and source code use the same storage and synchronization workflow.

| Crate | API |
| --- | --- |
| skald-vcs-core | Object model, hashing, storage, verification, and protocol types |
| skald-vcs-client | Working copies, index, history, merge, and HTTPS synchronization |
| skald-vcs-server | Hosted repositories, atomic references, quota hooks, and advisory locks |
| skald-vcs-ffi | C ABI built as a shared library and a static library |
| skald-vcs-cli | The skald command-line client |

## Install

Use a native release for your architecture when available. Put skald on your PATH. The SDK archive also contains the generated header, library, examples, and license. See the [native SDK guide](docs/native.md).

To build from source, install a current stable Rust toolchain and a native C toolchain. Run these commands from this workspace.

~~~sh
cargo build --workspace --release --locked
cargo install --path crates/cli --locked
cargo test --workspace --locked
~~~

Rust applications can use the client or server crate through a Cargo path dependency.

## Local workflow

~~~sh
skald init
skald add .
skald commit --author "Dev Name" -m "Add project assets"
skald branch development
skald switch development
skald status
skald log
skald diff --staged
skald verify
~~~

See the [CLI reference](docs/cli.md), [repository format](docs/format.md), [protocol reference](docs/protocol-v1.md), and [embedding guide](docs/hosting.md). See the [benchmark](docs/benchmarks.md) for measured storage and transfer comparisons. The workspace can be moved to its own repository and built independently.

A normal clone includes every advertised branch and tag with its full history. Use `skald clone URL --include Assets/Characters` to download file content selectively and `skald workspace` to change the selection later. Use UTF-8 portable paths and regular files. Symlinks, special files, submodules, and empty directories are unsupported. Merges report unresolved binary and overlapping text changes without replacing the local version of a conflicted file.
