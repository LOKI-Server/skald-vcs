use super::{Id, MAX_OBJECT, read_bounded, sync_directory};
use anyhow::{Context, Result};
use std::{
    fmt::Debug,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// A repository-scoped backend for encoded immutable objects.
///
/// A backend must bound reads to MAX_OBJECT + 1, publish complete objects
/// atomically and never overwrite an existing object. A shared physical backend
/// must scope contains, enumeration, and reads to the repository.
pub trait ObjectStorage: Debug + Send + Sync {
    fn contains(&self, id: Id) -> bool;
    fn read_encoded(&self, id: Id) -> Result<Vec<u8>>;
    fn insert_encoded(&self, id: Id, bytes: &[u8]) -> Result<bool>;
    fn list(&self) -> Result<Vec<Id>>;
    fn modified(&self, id: Id) -> Result<SystemTime>;
    fn remove(&self, id: Id) -> Result<()>;
}

#[derive(Debug)]
pub struct Filesystem {
    root: PathBuf,
}

impl Filesystem {
    pub fn new(path: &Path) -> Self {
        Self {
            root: path.to_owned(),
        }
    }

    fn path(&self, id: Id) -> PathBuf {
        let name = id.to_string();
        self.root.join(&name[..3]).join(&name[3..])
    }
}

impl ObjectStorage for Filesystem {
    fn contains(&self, id: Id) -> bool {
        self.path(id).is_file()
    }

    fn read_encoded(&self, id: Id) -> Result<Vec<u8>> {
        read_bounded(
            fs::File::open(self.path(id)).with_context(|| format!("missing object {id}"))?,
            MAX_OBJECT + 1,
        )
    }

    fn insert_encoded(&self, id: Id, bytes: &[u8]) -> Result<bool> {
        let path = self.path(id);
        let parent = path.parent().context("missing object shard")?;
        match fs::create_dir(parent) {
            Ok(()) => sync_directory(&self.root)?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(bytes)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(&path) {
            Ok(_) => {
                sync_directory(parent)?;
                Ok(true)
            }
            Err(e) if e.error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn list(&self) -> Result<Vec<Id>> {
        let mut ids = Vec::new();
        for shard in fs::read_dir(&self.root)? {
            let shard = shard?;
            let name = shard.file_name();
            let Some(name) = name.to_str().filter(|s| s.len() == 3) else {
                continue;
            };
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(shard.path())? {
                let entry = entry?;
                if let Some(suffix) = entry.file_name().to_str()
                    && let Ok(id) = format!("{name}{suffix}").parse()
                    && entry.file_type()?.is_file()
                {
                    ids.push(id);
                }
            }
        }
        Ok(ids)
    }

    fn modified(&self, id: Id) -> Result<SystemTime> {
        Ok(self.path(id).metadata()?.modified()?)
    }

    fn remove(&self, id: Id) -> Result<()> {
        Ok(fs::remove_file(self.path(id))?)
    }
}
