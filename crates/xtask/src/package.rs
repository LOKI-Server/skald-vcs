use anyhow::{Context, Result, bail, ensure};
use clap::Args;
use flate2::{Compression, write::GzEncoder};
use std::{
    fs::{self, File, Metadata},
    io,
    path::{Path, PathBuf},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

#[derive(Args)]
pub struct Options {
    /// The Rust target triple used for the release build.
    target: String,
    /// Directory containing the compiled binaries and libraries.
    #[arg(long)]
    build_dir: Option<PathBuf>,
    /// Cargo's target directory when --build-dir is omitted.
    #[arg(long, default_value = "target")]
    target_dir: PathBuf,
    /// Directory for the completed archive.
    #[arg(long, default_value = "dist")]
    output: PathBuf,
    /// Validate a release tag against the workspace version.
    #[arg(long, env = "SKALD_RELEASE_TAG")]
    tag: Option<String>,
}

enum Format {
    Zip,
    Tar,
}

pub(crate) struct Target {
    format: Format,
    executable: &'static str,
    libraries: &'static [&'static str],
}

impl Target {
    pub(crate) fn parse(triple: &str) -> Result<Self> {
        let (arch, platform) = triple
            .split_once('-')
            .context("expected a Rust target triple")?;
        ensure!(
            matches!(arch, "x86_64" | "aarch64"),
            "unsupported target {triple}"
        );
        Ok(match platform {
            "pc-windows-msvc" => Self {
                executable: "skald.exe",
                libraries: &["skald_vcs.dll", "skald_vcs.dll.lib", "skald_vcs.lib"],
                format: Format::Zip,
            },
            "apple-darwin" => Self {
                executable: "skald",
                libraries: &["libskald_vcs.dylib", "libskald_vcs.a"],
                format: Format::Tar,
            },
            "unknown-linux-gnu" | "unknown-linux-musl" => Self {
                executable: "skald",
                libraries: &["libskald_vcs.so", "libskald_vcs.a"],
                format: Format::Tar,
            },
            _ => bail!("unsupported target {triple}"),
        })
    }
}

struct Entry {
    name: String,
    source: Option<PathBuf>,
    mode: u32,
}

impl Entry {
    fn directory(name: &str) -> Self {
        Self {
            name: name.into(),
            source: None,
            mode: 0o755,
        }
    }

    fn file(name: &str, source: PathBuf, mode: u32) -> Result<Self> {
        ensure!(
            plain_metadata(&source)?.is_file(),
            "expected a regular file at {}",
            source.display()
        );
        Ok(Self {
            name: name.into(),
            source: Some(source),
            mode,
        })
    }
}

pub fn run(root: &Path, version: &str, options: &Options) -> Result<PathBuf> {
    if let Some(tag) = options.tag.as_deref().filter(|tag| !tag.is_empty()) {
        ensure!(
            tag == format!("v{version}"),
            "release tag must match the workspace version v{version}"
        );
    }
    let target = Target::parse(&options.target)?;
    let default_build = options.target_dir.join(&options.target).join("release");
    let build = options.build_dir.as_deref().unwrap_or(&default_build);
    Release::new(root, version, &options.target, target, build)?.write_archive(&options.output)
}

pub(crate) struct Release {
    format: Format,
    name: String,
    entries: Vec<Entry>,
}

impl Release {
    pub(crate) fn new(
        root: &Path,
        version: &str,
        triple: &str,
        target: Target,
        build: &Path,
    ) -> Result<Self> {
        let name = format!("skald-vcs-{version}-{triple}");
        let mut entries = vec![
            Entry::directory(&name),
            Entry::directory(&format!("{name}/bin")),
            Entry::directory(&format!("{name}/lib")),
            Entry::file(
                &format!("{name}/bin/{}", target.executable),
                build.join(target.executable),
                0o755,
            )?,
        ];
        for library in target.libraries {
            entries.push(Entry::file(
                &format!("{name}/lib/{library}"),
                build.join(library),
                0o644,
            )?);
        }
        for directory in ["include", "examples", "docs"] {
            collect(
                &root.join(directory),
                &format!("{name}/{directory}"),
                &mut entries,
            )?;
        }
        for file in ["README.md", "LICENSE"] {
            entries.push(Entry::file(
                &format!("{name}/{file}"),
                root.join(file),
                0o644,
            )?);
        }
        Ok(Self {
            format: target.format,
            name,
            entries,
        })
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn archive_name(&self) -> String {
        let ext = match self.format {
            Format::Zip => "zip",
            Format::Tar => "tar.gz",
        };
        format!("{}.{ext}", self.name)
    }

    pub(crate) fn write_archive(&self, directory: &Path) -> Result<PathBuf> {
        fs::create_dir_all(directory)?;
        let output = directory.join(self.archive_name());
        let mut temp = tempfile::NamedTempFile::new_in(directory)?;
        match self.format {
            Format::Zip => write_zip(temp.as_file_mut(), &self.entries)?,
            Format::Tar => write_tar(temp.as_file_mut(), &self.entries)?,
        }
        temp.as_file().sync_all()?;
        temp.persist(&output)
            .with_context(|| format!("could not publish {}", output.display()))?;
        Ok(output)
    }

    pub(crate) fn write_tree(&self, directory: &Path) -> Result<()> {
        for entry in &self.entries {
            let destination = directory.join(&entry.name);
            if let Some(source) = &entry.source {
                fs::copy(source, &destination)
                    .with_context(|| format!("could not copy {}", source.display()))?;
            } else {
                fs::create_dir(&destination)?;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&destination, fs::Permissions::from_mode(entry.mode))?;
            }
        }
        Ok(())
    }
}

fn plain_metadata(path: &Path) -> Result<Metadata> {
    let metadata = path
        .symlink_metadata()
        .with_context(|| format!("could not read {}", path.display()))?;
    ensure!(
        !metadata.is_symlink(),
        "release inputs cannot be symbolic links: {}",
        path.display()
    );
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        ensure!(
            metadata.file_attributes() & 0x400 == 0,
            "release inputs cannot be reparse points: {}",
            path.display()
        );
    }
    Ok(metadata)
}

fn collect(directory: &Path, name: &str, entries: &mut Vec<Entry>) -> Result<()> {
    ensure!(
        plain_metadata(directory)?.is_dir(),
        "expected a directory at {}",
        directory.display()
    );
    entries.push(Entry::directory(name));
    let mut children: Vec<_> = fs::read_dir(directory)?.collect::<io::Result<_>>()?;
    children.sort_unstable_by_key(|entry| entry.file_name());
    for child in children {
        let filename = child.file_name();
        let filename = filename
            .to_str()
            .context("archive entries require UTF-8 names")?;
        ensure!(
            !filename.contains(['\\', ':']),
            "invalid archive entry name {filename}"
        );
        let path = child.path();
        let name = format!("{name}/{filename}");
        if plain_metadata(&path)?.is_dir() {
            collect(&path, &name, entries)?;
        } else {
            entries.push(Entry::file(&name, path, 0o644)?);
        }
    }
    Ok(())
}

fn write_zip(file: &mut File, entries: &[Entry]) -> Result<()> {
    let mut archive = ZipWriter::new(file);
    for entry in entries {
        let options = SimpleFileOptions::default().unix_permissions(entry.mode);
        if let Some(source) = &entry.source {
            let mut input = File::open(source)?;
            archive.start_file(
                &entry.name,
                options
                    .compression_method(CompressionMethod::Deflated)
                    .large_file(input.metadata()?.len() >= u32::MAX as u64),
            )?;
            io::copy(&mut input, &mut archive)?;
        } else {
            archive.add_directory(&entry.name, options)?;
        }
    }
    archive.finish()?;
    Ok(())
}

fn write_tar(file: &mut File, entries: &[Entry]) -> Result<()> {
    let mut archive = tar::Builder::new(GzEncoder::new(file, Compression::default()));
    for entry in entries {
        let mut header = tar::Header::new_gnu();
        header.set_mode(entry.mode);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);
        if let Some(source) = &entry.source {
            let input = File::open(source)?;
            header.set_size(input.metadata()?.len());
            header.set_entry_type(tar::EntryType::Regular);
            archive.append_data(&mut header, &entry.name, input)?;
        } else {
            header.set_size(0);
            header.set_entry_type(tar::EntryType::Directory);
            archive.append_data(&mut header, &entry.name, io::empty())?;
        }
    }
    archive.into_inner()?.finish()?;
    Ok(())
}
