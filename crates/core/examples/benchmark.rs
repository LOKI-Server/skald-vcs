use anyhow::{Context, Result, ensure};
use skald_vcs_core::client::Repository;
use skald_vcs_core::{Id, Settings, Store};
use std::{
    collections::HashSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Instant,
};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().context("provide a source directory")?);
    let git = args.next().map(PathBuf::from);
    ensure!(
        args.next().is_none(),
        "usage: benchmark SOURCE [GIT_EXECUTABLE]"
    );
    let files = collect(&source)?;
    ensure!(!files.is_empty(), "no source files to benchmark");
    let changed_path = files[0].strip_prefix(&source)?.to_owned();
    let mut asset = vec![0; 32 * 1024 * 1024];
    let mut state = 0x5b16978d_u32;
    for byte in &mut asset {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    measure(
        "source",
        git.as_deref(),
        |dest| {
            for file in &files {
                let output = dest.join(file.strip_prefix(&source)?);
                fs::create_dir_all(output.parent().context("file parent")?)?;
                fs::copy(file, output)?;
            }
            Ok(())
        },
        |dest| {
            let path = dest.join(&changed_path);
            let mut data = b"// Benchmark edit\n".to_vec();
            data.extend(fs::read(&path)?);
            Ok(fs::write(path, data)?)
        },
    )?;
    measure(
        "binary",
        git.as_deref(),
        |dest| Ok(fs::write(dest.join("asset.bin"), &asset)?),
        |dest| {
            let path = dest.join("asset.bin");
            let mut shifted = fs::read(&path)?;
            shifted.splice(2_000_000..2_000_000, [0xff; 31]);
            Ok(fs::write(path, shifted)?)
        },
    )
}

fn collect(source: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![source.to_owned()];
    let mut total = 0u64;
    while let Some(dir) = dirs.pop() {
        for child in fs::read_dir(dir)? {
            let child = child?;
            if child.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = child.path();
            let kind = child.file_type()?;
            if kind.is_dir()
                && !matches!(child.file_name().to_str(), Some("target" | "node_modules"))
            {
                dirs.push(path);
            } else if kind.is_file()
                && matches!(
                    path.extension().and_then(|e| e.to_str()),
                    Some("rs" | "js" | "toml" | "md" | "sql" | "css" | "html")
                )
            {
                total += child.metadata()?.len();
                ensure!(
                    total <= 16 * 1024 * 1024 && files.len() < 2_000,
                    "source fixture exceeds benchmark bounds"
                );
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn measure(
    name: &str,
    git: Option<&Path>,
    setup: impl Fn(&Path) -> Result<()>,
    change: impl Fn(&Path) -> Result<()>,
) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let dir = temp.path().join("native");
    let repo = Repository::init(&dir, Settings::default())?;
    setup(&dir)?;
    let run = |title: &str| -> Result<(Id, f64)> {
        let start = Instant::now();
        repo.stage(&[".".into()])?;
        Ok((
            repo.commit("Benchmark", title)?,
            start.elapsed().as_secs_f64(),
        ))
    };
    let (first, t1) = run("initial")?;
    let store = Store::open(&dir.join(".skald/objects"), Settings::default())?;
    let first_disk = bytes(&dir.join(".skald/objects"))?;
    let first_objects = store.graph([first])?;
    change(&dir)?;
    let (second, t2) = run("update")?;
    let second_disk = bytes(&dir.join(".skald/objects"))?;
    let second_objects = store.graph([second])?;
    let payload = |ids: &HashSet<Id>| -> Result<usize> {
        ids.iter()
            .try_fold(0, |sum, id| Ok(sum + store.read(*id)?.len()))
    };
    let first_wire = payload(&first_objects)?;
    let delta_wire = payload(&second_objects.difference(&first_objects).copied().collect())?;
    println!("{name},Skald,{first_disk},{second_disk},{first_wire},{delta_wire},{t1:.3},{t2:.3}");
    if let Some(git) = git {
        let dir = temp.path().join("comparison");
        fs::create_dir(&dir)?;
        let hooks = temp.path().join("empty-hooks");
        fs::create_dir(&hooks)?;
        let git_command = || {
            let mut command = Command::new(git);
            for (name, _) in std::env::vars_os() {
                if name
                    .to_string_lossy()
                    .to_ascii_uppercase()
                    .starts_with("GIT_")
                {
                    command.env_remove(name);
                }
            }
            command
                .current_dir(&dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env(
                    "GIT_CONFIG_GLOBAL",
                    if cfg!(windows) { "NUL" } else { "/dev/null" },
                )
                .env("GIT_TERMINAL_PROMPT", "0")
                .args([
                    "-c",
                    "core.autocrlf=false",
                    "-c",
                    "user.name=Benchmark",
                    "-c",
                    "user.email=bench@example.invalid",
                    "-c",
                ])
                .arg(format!("core.hooksPath={}", hooks.display()));
            command
        };
        let run_git = |args: &[&str]| -> Result<String> {
            let out = git_command().args(args).output()?;
            ensure!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            Ok(String::from_utf8(out.stdout)?.trim().to_owned())
        };
        let pack = |revs: &str| -> Result<usize> {
            let mut child = git_command()
                .args([
                    "pack-objects",
                    "--revs",
                    "--stdout",
                    "--thin",
                    "--delta-base-offset",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            child
                .stdin
                .take()
                .context("missing pack stdin")?
                .write_all(revs.as_bytes())?;
            let out = child.wait_with_output()?;
            ensure!(out.status.success(), "git pack-objects failed");
            Ok(out.stdout.len())
        };
        let commit = |title| -> Result<(String, f64)> {
            let start = Instant::now();
            run_git(&["add", "."])?;
            run_git(&["commit", "-qm", title])?;
            let time = start.elapsed().as_secs_f64();
            Ok((run_git(&["rev-parse", "HEAD"])?, time))
        };
        run_git(&["init", "-q", "-b", "main"])?;
        setup(&dir)?;
        let (first, t1) = commit("initial")?;
        run_git(&["gc", "--quiet"])?;
        let first_disk = bytes(&dir.join(".git/objects"))?;
        let first_wire = pack(&format!("{first}\n"))?;
        change(&dir)?;
        let (second, t2) = commit("update")?;
        run_git(&["gc", "--quiet"])?;
        let second_disk = bytes(&dir.join(".git/objects"))?;
        let delta_wire = pack(&format!("{second}\n^{first}\n"))?;
        println!("{name},Git,{first_disk},{second_disk},{first_wire},{delta_wire},{t1:.3},{t2:.3}");
    }
    Ok(())
}

fn bytes(dir: &Path) -> Result<u64> {
    fs::read_dir(dir)?.try_fold(0, |sum, entry| {
        let entry = entry?;
        Ok(sum
            + if entry.file_type()?.is_dir() {
                bytes(&entry.path())?
            } else {
                entry.metadata()?.len()
            })
    })
}
