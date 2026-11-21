
# Embedding and release

Depend on skald-vcs-server and skald-vcs-core from Rust. A host allocates a private filesystem directory for each repository and creates a Hosted handle with its default branch. The host maps public repository identities to those directories. It authenticates requests and checks read, write, or management permissions before calling Hosted methods. Use a TLS frontend and keep the repository directories inaccessible to untrusted local writers.

Advertise Hosted::refs, negotiate through Hosted::missing, serve binary objects via Hosted::store and Store::read, and validate uploads through Hosted::receive. Supply a quota hook to receive. Hosted::update compares the expected reference and verifies the history before publishing the replacement. Treat Conflict as HTTP 409. The library serializes mutations through filesystem locks and stores refs and advisory locks in an atomically replaced record. Account identity, authorization, document review policy, and transport hosting belong to the embedding app. The filesystem store implements the repository-scoped ObjectStorage interface. Store validates and encodes logical objects independently of its backend. A future shared backend must preserve per-repository visibility, atomic publication, bounded reads, and safe collection.

LOKI embeds the server crate directly. The dashboard uses the existing session and CSRF machinery. Repository ownership, grants, and logical quotas are in SQLite. Repository objects and refs are under skald-repos in the installation data directory. Each upload is reserved against storage before the file is written, so the quota survives interruptions. A failed write can leave a conservative reservation that a retry or later GC resolves. Backups preserve native object files, refs, and database metadata. Restore verifies object integrity before installing a backup.

## Packaging and versions

The standalone workspace contains its release workflow under .github/workflows. Copy the entire directory when moving to a standalone repository. The workflow runs tests and builds native archives for x86-64 and ARM64 on Windows, Linux, and macOS. Trigger manually for build artifacts or create a matching version tag to publish release archives in the configured standalone repository. Linux builds use the runner's glibc baseline. On Windows the SDK uses the MSVC toolchain.

From the workspace root, run `just dist` to build the CLI and native SDK for
the host. Pass a Rust target triple to build for another platform when its
toolchain is installed.

~~~sh
just dist
just dist x86_64-pc-windows-msvc
~~~

The command writes a ready-to-use folder and its matching archive under
`target/dist/<target>/`. Each release includes the CLI, shared and static
libraries, C header, examples, documentation, and license. The Windows
release also includes the DLL import library. To run the same build without
Just, use `cargo xtask dist`.

To package binaries that have already been built, use the lower-level
packaging command with their target triple.

~~~sh
cargo build --release --locked --target x86_64-pc-windows-msvc -p skald-vcs-cli -p skald-vcs-ffi
cargo xtask package x86_64-pc-windows-msvc
~~~

For `cargo xtask package`, the archive goes to `dist/` by default. Use
`--target-dir` for a custom Cargo target directory, `--build-dir` for a
directory that already contains the release binaries, and `--output` for a
different archive directory. Pass `--tag` to check that the release tag
matches the workspace version.

Maintain SemVer for the Rust crates. Use explicit ABI and protocol versions because those consumers may upgrade separately. Before publishing crates, check the generated header and test the runtime examples, then publish core first, followed by client and server, and finally the CLI. The FFI crate is built from source for the native SDK archives. crates.io and release publishing require the project's account and credentials.
