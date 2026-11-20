use crate::package::{Release, Target};
use anyhow::{Context, Result, ensure};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};

pub fn run(root: &Path, version: &str, target: Option<String>) -> Result<(PathBuf, PathBuf)> {
    let triple = match target.filter(|value| !value.is_empty()) {
        Some(target) => target,
        None => host_target()?,
    };
    let platform = Target::parse(&triple)?;
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let target_dir = root.join("target");
    let status = Command::new(cargo)
        .current_dir(root)
        .args([
            "build",
            "--locked",
            "--release",
            "--target",
            &triple,
            "--target-dir",
        ])
        .arg(&target_dir)
        .args(["-p", "loki-vcs-cli", "-p", "loki-vcs-ffi"])
        .status()
        .context("could not start the release build")?;
    ensure!(status.success(), "release build failed for {triple}");

    let release = Release::new(
        root,
        version,
        &triple,
        platform,
        &target_dir.join(&triple).join("release"),
    )?;
    let dist = target_dir.join("dist");
    fs::create_dir_all(&dist)?;
    let stage = tempfile::Builder::new()
        .prefix(".lvcs-dist-")
        .tempdir_in(&dist)?;
    let next = stage.path().join("next");
    fs::create_dir(&next)?;
    release.write_tree(&next)?;
    release.write_archive(&next)?;
    let dest = dist.join(triple);
    replace_target(stage, &dest)?;
    Ok((dest.join(release.name()), dest.join(release.archive_name())))
}

fn host_target() -> Result<String> {
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(rustc)
        .arg("-vV")
        .output()
        .context("could not start rustc")?;
    ensure!(output.status.success(), "rustc -vV failed");
    String::from_utf8(output.stdout)?
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
        .context("rustc did not report its host target")
}

fn replace_target(stage: tempfile::TempDir, dest: &Path) -> Result<()> {
    let previous = stage.path().join("previous");
    let has_previous = match fs::symlink_metadata(dest) {
        Ok(metadata) => {
            ensure!(
                metadata.is_dir() && !metadata.is_symlink(),
                "{} must be a directory",
                dest.display()
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    metadata.file_attributes() & 0x400 == 0,
                    "{} must not be a reparse point",
                    dest.display()
                );
            }
            fs::rename(dest, &previous)
                .with_context(|| format!("could not set aside {}", dest.display()))?;
            true
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    if let Err(error) = fs::rename(stage.path().join("next"), dest) {
        if has_previous && let Err(rollback) = fs::rename(&previous, dest) {
            let backup = stage.keep().join("previous");
            return Err(error).context(format!("could not publish {} or restore its previous contents ({rollback}); previous release is at {}", dest.display(), backup.display()));
        }
        return Err(error).with_context(|| format!("could not publish {}", dest.display()));
    }
    Ok(())
}
