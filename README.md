
# Loki VCS 0.1

Loki VCS is an offline-first version control system written in Rust. It stores revisions and directory snapshots by content, uses content-defined BLAKE3 chunks for all files, and compresses useful chunks with Zstandard. Binary assets and source code use the same storage and synchronization workflow.

| Crate | API |
| --- | --- |
| loki-vcs-core | Object model, hashing, storage, verification, and protocol types |
| loki-vcs-client | Working copies, index, history, merge, and HTTPS synchronization |
| loki-vcs-server | Hosted repositories, atomic references, quota hooks, and advisory locks |
| loki-vcs-ffi | C ABI built as a shared library and a static library |
| loki-vcs-cli | The lvcs command-line client |

## Install

Install the published CLI with Cargo.

~~~sh
cargo install loki-vcs-cli --locked
~~~

Use a native release for your architecture when available. Put lvcs on your PATH. The SDK archive also contains the generated header, library, examples, and license. See the [native SDK guide](docs/native.md).

To build from source, install a current stable Rust toolchain and a native C toolchain. Run these commands from this workspace.

~~~sh
cargo build --workspace --release --locked
cargo install --path crates/cli --locked
cargo test --workspace --locked
~~~

Rust applications can add the published client or server crate with `cargo add loki-vcs-client@0.1.0` or `cargo add loki-vcs-server@0.1.0`.

## Local workflow

~~~sh
lvcs init
lvcs add .
lvcs commit --author "Dev Name" -m "Add project assets"
lvcs branch development
lvcs switch development
lvcs status
lvcs log
lvcs diff --staged
lvcs verify
~~~

See the [CLI reference](docs/cli.md), [repository format](docs/format.md), [protocol reference](docs/protocol-v1.md), and [embedding guide](docs/hosting.md). See the [benchmark](docs/benchmarks.md) for measured storage and transfer comparisons. The workspace can be moved to its own repository and built independently.

Complete clones include every advertised branch and tag, with their full history. Sparse and partial clones are unsupported in format and protocol v1. Use UTF-8 portable paths and regular files. Symlinks, special files, submodules, and empty directories are unsupported. Merges report unresolved binary and overlapping text changes without replacing the local version of a conflicted file.
