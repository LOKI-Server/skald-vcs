set windows-shell := ["powershell.exe", "-NoLogo", "-NoProfile", "-Command"]

# Build the CLI and SDK and prepare target/dist for the host or a Rust target triple.
dist target="":
    cargo xtask dist {{quote(target)}}

# Run the workspace tests.
test:
    cargo test --locked --workspace
