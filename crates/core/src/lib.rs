use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fmt, fs,
    io::{BufReader, Read, Write},
    path::Path,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};
use unicode_normalization::UnicodeNormalization;

pub const VERSION: u32 = 1;
pub const MAX_OBJECT: usize = 8 * 1024 * 1024;
pub const MAX_GRAPH: usize = 500_000;
pub const MAX_DEPTH: usize = 64;
pub const MAX_FILES: usize = 250_000;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Kind {
    Chunk,
    File,
    Tree,
    Revision,
}

impl Kind {
    fn prefix(self) -> char {
        match self {
            Self::Chunk => 'c',
            Self::File => 'f',
            Self::Tree => 't',
            Self::Revision => 'r',
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Id {
    kind: Kind,
    digest: [u8; 32],
}

impl Id {
    pub fn of(kind: Kind, raw: &[u8]) -> Self {
        Self {
            kind,
            digest: *blake3::hash(raw).as_bytes(),
        }
    }

    pub fn kind(self) -> Kind {
        self.kind
    }
}

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.kind.prefix(),
            blake3::Hash::from_bytes(self.digest).to_hex()
        )
    }
}

impl FromStr for Id {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        ensure!(s.len() == 65 && s.is_ascii(), "invalid object ID");
        let kind = match s.as_bytes()[0] {
            b'c' => Kind::Chunk,
            b'f' => Kind::File,
            b't' => Kind::Tree,
            b'r' => Kind::Revision,
            _ => bail!("invalid object kind"),
        };
        ensure!(
            s[1..]
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid object ID"
        );
        Ok(Self {
            kind,
            digest: *blake3::Hash::from_hex(&s[1..])?.as_bytes(),
        })
    }
}

impl TryFrom<String> for Id {
    type Error = anyhow::Error;
    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Id> for String {
    fn from(id: Id) -> Self {
        id.to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub id: Id,
    pub size: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub size: u64,
    pub chunks: Vec<Chunk>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: Id,
    pub executable: bool,
}

pub type Tree = BTreeMap<String, Entry>;
pub type Snapshot = BTreeMap<String, Entry>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revision {
    pub parents: Vec<Id>,
    pub root: Id,
    pub author: String,
    pub timestamp: i64,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Settings {
    pub min: usize,
    pub average: usize,
    pub max: usize,
    pub compression: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            min: 64 * 1024,
            average: 256 * 1024,
            max: 1024 * 1024,
            compression: 3,
        }
    }
}

impl Settings {
    pub fn validate(self) -> Result<()> {
        ensure!(
            self.min >= 1024
                && self.min < self.average
                && self.average < self.max
                && self.average.is_power_of_two()
                && self.max <= MAX_OBJECT
                && (-5..=19).contains(&self.compression),
            "invalid chunk or compression settings"
        );
        Ok(())
    }
}

pub fn component(name: &str) -> Result<()> {
    let stem = name.split('.').next().unwrap_or("");
    let upper = stem.to_ascii_uppercase();
    ensure!(
        !name.is_empty()
            && name.len() <= 255
            && name.nfc().eq(name.chars())
            && name != "."
            && name != ".."
            && !name.to_ascii_lowercase().starts_with(".skald-")
            && !name.eq_ignore_ascii_case(".skald")
            && !name
                .as_bytes()
                .windows(2)
                .any(|w| w[0] == b'~' && w[1].is_ascii_digit())
            && !name.ends_with(['.', ' '])
            && !name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
            && !matches!(
                upper.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            && !["COM", "LPT"]
                .iter()
                .any(|p| upper.strip_prefix(p).is_some_and(|s| matches!(
                    s,
                    "1" | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â¹"
                        | "ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â²"
                        | "ÃƒÆ’Ã†â€™Ãƒâ€ Ã¢â‚¬â„¢ÃƒÆ’Ã‚Â¢ÃƒÂ¢Ã¢â‚¬Å¡Ã‚Â¬Ãƒâ€¦Ã‚Â¡ÃƒÆ’Ã†â€™ÃƒÂ¢Ã¢â€šÂ¬Ã…Â¡ÃƒÆ’Ã¢â‚¬Å¡Ãƒâ€šÃ‚Â³"
                ))),
        "invalid portable path component {name:?}"
    );
    Ok(())
}

pub fn valid_path(path: &str) -> Result<()> {
    ensure!(
        path.len() <= 1024 && path.split('/').count() <= MAX_DEPTH,
        "path exceeds limits"
    );
    for part in path.split('/') {
        component(part)?;
    }
    Ok(())
}

pub fn reference(name: &str) -> Result<()> {
    ensure!(
        name.len() <= 160
            && name.split('/').all(|p| !p.is_empty()
                && p.as_bytes()[0].is_ascii_alphanumeric()
                && p.bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"._-".contains(&c))
                && !p.ends_with('.')
                && !p.contains("..")),
        "invalid reference name"
    );
    Ok(())
}

pub fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec(value)?;
    ensure!(bytes.len() <= MAX_OBJECT, "object exceeds size limit");
    Ok(bytes)
}

fn decode<T: DeserializeOwned + Serialize>(raw: &[u8]) -> Result<T> {
    let value = serde_json::from_slice::<T>(raw)?;
    ensure!(canonical(&value)? == raw, "noncanonical object");
    Ok(value)
}

pub enum Object {
    Chunk,
    File(Manifest),
    Tree(Tree),
    Revision(Revision),
}

impl Object {
    pub fn parse(id: Id, raw: &[u8]) -> Result<Self> {
        ensure!(
            raw.len() <= MAX_OBJECT && Id::of(id.kind, raw) == id,
            "object hash or size mismatch: {id}"
        );
        Ok(match id.kind {
            Kind::Chunk => {
                ensure!(!raw.is_empty(), "empty chunk");
                Self::Chunk
            }
            Kind::File => {
                let file: Manifest = decode(raw)?;
                let mut size = 0u64;
                for chunk in &file.chunks {
                    ensure!(
                        chunk.id.kind == Kind::Chunk
                            && (1..=MAX_OBJECT as u32).contains(&chunk.size),
                        "invalid chunk descriptor"
                    );
                    size += u64::from(chunk.size);
                }
                ensure!(size == file.size, "invalid manifest size");
                Self::File(file)
            }
            Kind::Tree => {
                let entries: Tree = decode(raw)?;
                let mut names = HashSet::new();
                for (name, entry) in &entries {
                    component(name)?;
                    ensure!(
                        names.insert(name.to_uppercase().nfc().collect::<String>()),
                        "case collision in tree"
                    );
                    ensure!(
                        entry.id.kind == Kind::File
                            || (entry.id.kind == Kind::Tree && !entry.executable),
                        "invalid tree entry"
                    );
                }
                Self::Tree(entries)
            }
            Kind::Revision => {
                let revision: Revision = decode(raw)?;
                ensure!(
                    revision.root.kind == Kind::Tree
                        && revision.parents.len() <= 2
                        && revision.parents.iter().all(|p| p.kind == Kind::Revision)
                        && (revision.parents.len() < 2
                            || revision.parents[0] != revision.parents[1])
                        && !revision.author.trim().is_empty()
                        && revision.author.len() <= 256
                        && !revision.author.chars().any(char::is_control)
                        && !revision.message.trim().is_empty()
                        && revision.message.len() <= 64 * 1024,
                    "invalid revision"
                );
                Self::Revision(revision)
            }
        })
    }

    pub fn children(&self) -> Vec<Id> {
        match self {
            Self::Chunk => Vec::new(),
            Self::File(file) => file.chunks.iter().map(|c| c.id).collect(),
            Self::Tree(tree) => tree.values().map(|e| e.id).collect(),
            Self::Revision(revision) => revision
                .parents
                .iter()
                .copied()
                .chain([revision.root])
                .collect(),
        }
    }
}

pub fn read_bounded(reader: impl Read, max: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(max as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= max, "input exceeds its size limit");
    Ok(bytes)
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .context("metadata requires a parent directory")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut temp, value)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    sync_directory(parent)
}

pub fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

pub fn disk_lock(path: &Path) -> Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}

#[derive(Clone, Debug)]
pub struct Store {
    backend: Arc<dyn ObjectStorage>,
    settings: Settings,
}

impl Store {
    pub fn create(root: &Path, settings: Settings) -> Result<Self> {
        settings.validate()?;
        fs::create_dir_all(root)?;
        Self::open(root, settings)
    }

    pub fn open(root: &Path, settings: Settings) -> Result<Self> {
        ensure!(root.is_dir(), "missing object directory");
        Self::with_backend(Arc::new(Filesystem::new(root)), settings)
    }

    pub fn with_backend(backend: Arc<dyn ObjectStorage>, settings: Settings) -> Result<Self> {
        settings.validate()?;
        Ok(Self { backend, settings })
    }

    pub fn contains(&self, id: Id) -> bool {
        self.backend.contains(id)
    }

    pub fn put(&self, kind: Kind, raw: &[u8]) -> Result<Id> {
        let id = Id::of(kind, raw);
        self.receive(id, raw)?;
        Ok(id)
    }

    pub fn receive(&self, id: Id, raw: &[u8]) -> Result<()> {
        Object::parse(id, raw)?;
        if self.contains(id) {
            self.read(id)?;
            return Ok(());
        }
        let mut encoded = Vec::with_capacity(raw.len() + 1);
        if id.kind == Kind::Chunk && raw.len() >= 256 {
            let sample = &raw[..raw.len().min(32 * 1024)];
            let estimate = zstd::bulk::compress(sample, 1)?;
            if estimate.len() + sample.len() / 20 < sample.len() {
                let compressed = zstd::bulk::compress(raw, self.settings.compression)?;
                if compressed.len() + raw.len() / 20 < raw.len() {
                    encoded.push(1);
                    encoded.extend(compressed);
                }
            }
        }
        if encoded.is_empty() {
            encoded.push(0);
            encoded.extend_from_slice(raw);
        }
        if !self.backend.insert_encoded(id, &encoded)? {
            self.read(id)?;
        }
        Ok(())
    }

    fn read_verified(&self, id: Id) -> Result<(Vec<u8>, Object)> {
        let encoded = self.backend.read_encoded(id)?;
        ensure!(
            encoded.len() <= MAX_OBJECT + 1,
            "stored object exceeds limit"
        );
        let (&codec, input) = encoded.split_first().context("empty stored object")?;
        let raw = match codec {
            0 => input.to_vec(),
            1 if id.kind == Kind::Chunk => {
                let mut decoder = zstd::stream::read::Decoder::with_buffer(input)?;
                decoder.window_log_max(23)?;
                read_bounded(decoder, MAX_OBJECT)?
            }
            _ => bail!("unknown object encoding"),
        };
        let object = Object::parse(id, &raw)?;
        Ok((raw, object))
    }

    pub fn read(&self, id: Id) -> Result<Vec<u8>> {
        Ok(self.read_verified(id)?.0)
    }

    pub fn object(&self, id: Id) -> Result<Object> {
        Ok(self.read_verified(id)?.1)
    }

    pub fn revision(&self, id: Id) -> Result<Revision> {
        match self.object(id)? {
            Object::Revision(r) => Ok(r),
            _ => bail!("expected revision"),
        }
    }

    pub fn manifest(&self, id: Id) -> Result<Manifest> {
        match self.object(id)? {
            Object::File(f) => Ok(f),
            _ => bail!("expected file"),
        }
    }

    pub fn tree(&self, id: Id) -> Result<Tree> {
        match self.object(id)? {
            Object::Tree(t) => Ok(t),
            _ => bail!("expected directory"),
        }
    }

    pub fn write_file(&self, input: impl Read) -> Result<Id> {
        let mut input = BufReader::with_capacity(64 * 1024, input);
        let mut cursor = [0u8; 64 * 1024];
        let mut chunk = Vec::with_capacity(self.settings.max);
        let mut hash = 0u64;
        let avg = self.settings.average as u64;
        let mut manifest = Manifest {
            size: 0,
            chunks: Vec::new(),
        };
        // Gear rolling hash with stricter boundaries before the target size.
        loop {
            let n = input.read(&mut cursor)?;
            if n == 0 {
                break;
            }
            for byte in &cursor[..n] {
                chunk.push(*byte);
                let len = chunk.len();
                if len >= self.settings.min {
                    hash = hash.wrapping_shl(1).wrapping_add(GEAR[*byte as usize]);
                    let mask = if len < self.settings.average {
                        (avg << 1) - 1
                    } else {
                        (avg >> 1) - 1
                    };
                    if len == self.settings.max || hash & mask == 0 {
                        self.finish_chunk(&mut chunk, &mut manifest)?;
                        hash = 0;
                    }
                }
            }
        }
        if !chunk.is_empty() {
            self.finish_chunk(&mut chunk, &mut manifest)?;
        }
        self.put(Kind::File, &canonical(&manifest)?)
    }

    fn finish_chunk(&self, chunk: &mut Vec<u8>, manifest: &mut Manifest) -> Result<()> {
        ensure!(
            manifest.chunks.len() < MAX_OBJECT / 100,
            "manifest chunk limit reached"
        );
        let id = self.put(Kind::Chunk, chunk)?;
        manifest.size += chunk.len() as u64;
        manifest.chunks.push(Chunk {
            id,
            size: chunk.len() as u32,
        });
        chunk.clear();
        Ok(())
    }

    pub fn copy_file(&self, id: Id, mut output: impl Write) -> Result<()> {
        for chunk in self.manifest(id)?.chunks {
            let bytes = self.read(chunk.id)?;
            ensure!(bytes.len() == chunk.size as usize, "chunk size mismatch");
            output.write_all(&bytes)?;
        }
        Ok(())
    }

    pub fn snapshot(&self, commit: Option<Id>) -> Result<Snapshot> {
        let mut files = Snapshot::new();
        if let Some(commit) = commit {
            self.flatten(self.revision(commit)?.root, "", 0, &mut 0, &mut files)?;
        }
        Ok(files)
    }

    fn flatten(
        &self,
        id: Id,
        prefix: &str,
        depth: usize,
        visited: &mut usize,
        files: &mut Snapshot,
    ) -> Result<()> {
        ensure!(depth < MAX_DEPTH, "tree depth exceeds limit");
        for (name, entry) in self.tree(id)? {
            *visited += 1;
            ensure!(*visited <= MAX_FILES, "expanded tree exceeds entry limit");
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            ensure!(path.len() <= 1024, "expanded path exceeds limit");
            if entry.id.kind == Kind::Tree {
                self.flatten(entry.id, &path, depth + 1, visited, files)?;
            } else {
                files.insert(path, entry);
            }
        }
        Ok(())
    }

    pub fn write_snapshot(&self, files: &Snapshot) -> Result<Id> {
        ensure!(files.len() <= MAX_FILES, "snapshot exceeds file limit");
        let mut directories = BTreeMap::<String, Tree>::from([(String::new(), Tree::new())]);
        for (path, entry) in files {
            valid_path(path)?;
            ensure!(
                entry.id.kind == Kind::File,
                "snapshot requires file entries"
            );
            let (dir, file) = path.rsplit_once('/').unwrap_or(("", path));
            directories
                .entry(dir.to_owned())
                .or_default()
                .insert(file.to_owned(), *entry);
            let mut dir = dir;
            while let Some((parent, _)) = dir.rsplit_once('/') {
                directories.entry(parent.to_owned()).or_default();
                dir = parent;
            }
        }
        ensure!(
            files.len() + directories.len() <= MAX_FILES,
            "expanded tree exceeds entry limit"
        );
        while let Some((path, tree)) = directories.pop_last() {
            let id = self.put(Kind::Tree, &canonical(&tree)?)?;
            if path.is_empty() {
                return Ok(id);
            }
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", &path));
            ensure!(
                directories
                    .entry(parent.to_owned())
                    .or_default()
                    .insert(
                        name.to_owned(),
                        Entry {
                            id,
                            executable: false
                        }
                    )
                    .is_none(),
                "file and directory collide"
            );
        }
        unreachable!("the root directory is always present")
    }

    pub fn graph(&self, roots: impl IntoIterator<Item = Id>) -> Result<HashSet<Id>> {
        self.graph_since(roots, &HashSet::new())
    }

    /// Traverse and verify objects until reaching already-verified revisions.
    /// Callers must guarantee that every excluded root and its full closure remains present.
    pub fn graph_since(
        &self,
        roots: impl IntoIterator<Item = Id>,
        trusted: &HashSet<Id>,
    ) -> Result<HashSet<Id>> {
        self.walk_graph(roots, trusted, ChunkRead::Required)
    }

    /// Verify metadata and enumerate all referenced IDs without reading chunk payloads.
    pub fn metadata_graph(
        &self,
        roots: impl IntoIterator<Item = Id>,
        trusted: &HashSet<Id>,
    ) -> Result<HashSet<Id>> {
        self.walk_graph(roots, trusted, ChunkRead::Skip)
    }

    fn walk_graph(
        &self,
        roots: impl IntoIterator<Item = Id>,
        trusted: &HashSet<Id>,
        chunks: ChunkRead,
    ) -> Result<HashSet<Id>> {
        let mut seen = HashSet::new();
        let mut queued = HashSet::new();
        let mut pending = Vec::new();
        let mut sizes = HashMap::new();
        let mut enqueue = |id, pending: &mut Vec<Id>| -> Result<()> {
            if !trusted.contains(&id) && queued.insert(id) {
                ensure!(queued.len() <= MAX_GRAPH, "graph exceeds object limit");
                pending.push(id);
            }
            Ok(())
        };
        for id in roots {
            enqueue(id, &mut pending)?;
        }
        while let Some(id) = pending.pop() {
            seen.insert(id);
            if id.kind() == Kind::Chunk {
                let skip = match chunks {
                    ChunkRead::Required => false,
                    ChunkRead::Cached => !self.contains(id),
                    ChunkRead::Skip => true,
                };
                if skip {
                    continue;
                }
            }
            let (raw, object) = self.read_verified(id)?;
            match object {
                Object::Chunk => check_size(&mut sizes, id, raw.len() as u32)?,
                Object::File(file) => {
                    for chunk in file.chunks {
                        check_size(&mut sizes, chunk.id, chunk.size)?;
                        enqueue(chunk.id, &mut pending)?;
                    }
                }
                other => {
                    for child in other.children() {
                        enqueue(child, &mut pending)?;
                    }
                }
            }
        }
        Ok(seen)
    }

    pub fn history(
        &self,
        roots: impl IntoIterator<Item = Id>,
        limit: usize,
    ) -> Result<Vec<(Id, Revision)>> {
        let mut pending: std::collections::VecDeque<_> = roots.into_iter().collect();
        let mut seen = HashSet::new();
        let mut history = Vec::new();
        while history.len() < limit {
            let Some(id) = pending.pop_front() else { break };
            if seen.insert(id) {
                let commit = self.revision(id)?;
                pending.extend(commit.parents.iter().copied());
                history.push((id, commit));
            }
        }
        Ok(history)
    }

    pub fn is_ancestor(&self, ancestor: Id, head: Id) -> Result<bool> {
        let mut pending = vec![head];
        let mut seen = HashSet::new();
        while let Some(id) = pending.pop() {
            if id == ancestor {
                return Ok(true);
            }
            if seen.insert(id) {
                ensure!(seen.len() <= MAX_GRAPH, "history exceeds traversal limit");
                pending.extend(self.revision(id)?.parents);
            }
        }
        Ok(false)
    }

    // Validate each new tree's expanded size, depth and path length without
    // enumerating repeated directory instances or expanding a snapshot.
    pub fn validate_history(&self, head: Id, trusted: &HashSet<Id>) -> Result<()> {
        self.graph_since([head], trusted)?;
        let mut memo = HashMap::new();
        let mut seen = HashSet::new();
        let mut pending = vec![head];
        while let Some(id) = pending.pop() {
            if !trusted.contains(&id) && seen.insert(id) {
                let revision = self.revision(id)?;
                self.tree_shape(revision.root, 0, &mut memo)?;
                pending.extend(revision.parents);
            }
        }
        Ok(())
    }

    fn tree_shape(
        &self,
        id: Id,
        depth: usize,
        memo: &mut HashMap<Id, (usize, usize, usize)>,
    ) -> Result<(usize, usize, usize)> {
        ensure!(depth < MAX_DEPTH, "tree depth exceeds limit");
        if let Some(&shape) = memo.get(&id) {
            return Ok(shape);
        }
        let (mut entries, mut height, mut length) = (0usize, 0usize, 0usize);
        for (name, entry) in self.tree(id)? {
            entries += 1;
            if entry.id.kind == Kind::Tree {
                let (count, child_height, child_length) =
                    self.tree_shape(entry.id, depth + 1, memo)?;
                entries += count;
                height = height.max(child_height + 1);
                length = length.max(name.len() + 1 + child_length);
            } else {
                length = length.max(name.len());
            }
            ensure!(
                entries <= MAX_FILES && height < MAX_DEPTH && length <= 1024,
                "expanded tree exceeds limits"
            );
        }
        memo.insert(id, (entries, height, length));
        Ok((entries, height, length))
    }

    /// Export only committed object files. Atomic-write scratch files are ignored.
    pub fn backup_to(&self, destination: &Path) -> Result<()> {
        for id in self.object_ids()? {
            let name = id.to_string();
            let shard = destination.join(&name[..3]);
            fs::create_dir_all(&shard)?;
            fs::write(shard.join(&name[3..]), self.backend.read_encoded(id)?)?;
        }
        Ok(())
    }

    pub fn object_ids(&self) -> Result<Vec<Id>> {
        self.backend.list()
    }

    /// Caller must hold the same writer lock used when creating objects and modifying roots.
    pub fn collect(&self, roots: impl IntoIterator<Item = Id>, grace: Duration) -> Result<usize> {
        let live = self.graph(roots)?;
        self.collect_except(&live, grace)
    }

    /// Collect objects outside a caller-verified keep set under the writer lock.
    pub fn collect_except(&self, live: &HashSet<Id>, grace: Duration) -> Result<usize> {
        let mut removed = 0;
        for id in self.object_ids()? {
            if !live.contains(&id)
                && self
                    .backend
                    .modified(id)?
                    .elapsed()
                    .is_ok_and(|age| age >= grace)
            {
                self.backend.remove(id)?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    pub fn verify(&self, roots: impl IntoIterator<Item = Id>) -> Result<()> {
        self.verify_objects(&self.graph(roots)?)
    }

    /// Verify reachable metadata and cached chunks, allowing missing history payloads.
    /// A sparse client must separately verify the full closure of materialized files.
    pub fn verify_cached(&self, roots: impl IntoIterator<Item = Id>) -> Result<()> {
        let reachable = self.walk_graph(roots, &HashSet::new(), ChunkRead::Cached)?;
        self.verify_objects(&reachable)
    }

    fn verify_objects(&self, reachable: &HashSet<Id>) -> Result<()> {
        for id in self.object_ids()? {
            if !reachable.contains(&id) {
                self.read(id)?;
            }
        }
        Ok(())
    }
}

enum ChunkRead {
    Required,
    Cached,
    Skip,
}

fn check_size(sizes: &mut HashMap<Id, u32>, id: Id, size: u32) -> Result<()> {
    ensure!(
        *sizes.entry(id).or_insert(size) == size,
        "chunk size mismatch: {id}"
    );
    Ok(())
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

const GEAR: [u64; 256] = {
    let mut table = [0; 256];
    let mut i = 0;
    let mut seed = 0x243f6a8885a308d3u64;
    while i < table.len() {
        seed = seed.wrapping_add(0x9e3779b97f4a7c15);
        let a = (seed ^ (seed >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        let b = (a ^ (a >> 27)).wrapping_mul(0x94d049bb133111eb);
        table[i] = b ^ (b >> 31);
        i += 1;
    }
    table
};
pub mod protocol;
mod storage;
pub mod text;
pub use storage::{Filesystem, ObjectStorage};
