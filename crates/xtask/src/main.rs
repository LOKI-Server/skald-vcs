mod dist;
mod package;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::path::Path;

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Package the native SDK and CLI from an existing release build.
    Package(package::Options),
    /// Build and assemble a native distribution under target/dist.
    Dist {
        /// Rust target triple, or the host target when omitted.
        target: Option<String>,
    },
}

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .context("xtask is not in the workspace")?;
    match Cli::parse().command {
        Command::Package(options) => {
            let archive = package::run(root, env!("CARGO_PKG_VERSION"), &options)?;
            println!("{}", archive.display());
        }
        Command::Dist { target } => {
            let (directory, archive) = dist::run(root, env!("CARGO_PKG_VERSION"), target)?;
            println!("{}", directory.display());
            println!("{}", archive.display());
        }
    }
    Ok(())
}
