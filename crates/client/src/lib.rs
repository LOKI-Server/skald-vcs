mod network;
use anyhow::{Context, Result, bail, ensure};
use ignore::gitignore::GitignoreBuilder;
use loki_vcs_core::text::{file_text, unified_patch};
use loki_vcs_core::{
    Entry, Id, Kind, Revision, Settings, Snapshot, Store, atomic_json, canonical, disk_lock, now,
    reference, sync_directory, valid_path,
};
pub use network::Remote;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime},
};
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Head {
    Branch(String),
    Detached(Id),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct State {
    version: u32,
    head: Head,
    branches: BTreeMap<String, Option<Id>>,
    tags: BTreeMap<String, Id>,
    index: Snapshot,
    settings: Settings,
    remotes: BTreeMap<String, String>,
    fetched: BTreeMap<String, BTreeMap<String, Id>>,
    merge_parent: Option<Id>,
    conflicts: BTreeSet<String>,
}
impl State {
    fn tip(&self) -> Option<Id> {
        match &self.head {
            Head::Branch(name) => self.branches[name],
            Head::Detached(id) => Some(*id),
        }
    }
}
#[derive(Serialize, Deserialize)]
struct Checkout {
    old: Snapshot,
    new: Snapshot,
    state: State,
}
#[derive(Debug, Serialize)]
pub struct Status {
    pub branch: Option<String>,
    pub revision: Option<Id>,
    pub staged: BTreeMap<String, &'static str>,
    pub unstaged: BTreeMap<String, &'static str>,
    pub conflicts: BTreeSet<String>,
}
#[derive(Debug, Serialize)]
pub struct Difference {
    pub path: String,
    pub change: &'static str,
    pub text: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Merge {
    pub revision: Option<Id>,
    pub conflicts: BTreeSet<String>,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct Fingerprint {
    modified: SystemTime,
    size: u64,
    executable: bool,
    identity: [i64; 4],
}

impl Fingerprint {
    fn observed(m: &fs::Metadata, now: SystemTime) -> Option<Self> {
        let modified = m.modified().ok()?;
        // Avoid reusing observations made during a coarse filesystem timestamp tick.
        if now.duration_since(modified).ok()? < Duration::from_secs(2) {
            return None;
        }
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            [m.dev() as i64, m.ino() as i64, m.ctime(), m.ctime_nsec()]
        };
        #[cfg(windows)]
        let identity = {
            use std::os::windows::fs::MetadataExt;
            [m.creation_time() as i64, m.file_attributes() as i64, 0, 0]
        };
        #[cfg(not(any(unix, windows)))]
        let identity = return None;
        Some(Self {
            modified,
            size: m.len(),
            executable: executable(m),
            identity,
        })
    }
}

type StatusCache = BTreeMap<String, (Entry, Fingerprint)>;

pub struct Repository {
    root: PathBuf,
    meta: PathBuf,
}
impl Repository {
    pub fn init(path: impl AsRef<Path>, settings: Settings) -> Result<Self> {
        settings.validate()?;
        fs::create_dir_all(path.as_ref())?;
        let root = path.as_ref().canonicalize()?;
        let meta = root.join(".lvcs");
        ensure!(!meta.exists(), "a working copy already exists here");
        let staging = tempfile::Builder::new()
            .prefix(".lvcs-init-")
            .tempdir_in(&root)?;
        Store::create(&staging.path().join("objects"), settings)?;
        atomic_json(
            &staging.path().join("state.json"),
            &State {
                version: loki_vcs_core::VERSION,
                head: Head::Branch("main".into()),
                branches: BTreeMap::from([("main".into(), None)]),
                tags: BTreeMap::new(),
                index: Snapshot::new(),
                settings,
                remotes: BTreeMap::new(),
                fetched: BTreeMap::new(),
                merge_parent: None,
                conflicts: BTreeSet::new(),
            },
        )?;
        fs::rename(staging.path(), &meta)?;
        sync_directory(&root)?;
        Ok(Self { root, meta })
    }
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().canonicalize()?;
        let meta = root.join(".lvcs");
        ensure!(
            safe_metadata(&meta)?.is_some_and(|m| m.is_dir()),
            "no working copy at {}",
            root.display()
        );
        let repo = Self { root, meta };
        repo.with_state(|_, _| Ok(()))?;
        Ok(repo)
    }
    pub fn discover(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().canonicalize()?;
        for dir in root.ancestors() {
            if safe_metadata(&dir.join(".lvcs"))?.is_some() {
                return Self::open(dir);
            }
        }
        bail!("no .lvcs working copy found");
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    fn with_state<T>(&self, f: impl FnOnce(&mut State, &Store) -> Result<T>) -> Result<T> {
        let _lock = disk_lock(&self.meta.join("writer.lock"))?;
        let mut state: State = self
            .read_meta("state.json")?
            .context("missing working copy metadata")?;
        ensure!(
            state.version == loki_vcs_core::VERSION,
            "unsupported working copy version"
        );
        ensure!(
            match &state.head {
                Head::Branch(name) => state.branches.contains_key(name),
                Head::Detached(_) => true,
            },
            "invalid working copy head"
        );
        let store = Store::open(&self.meta.join("objects"), state.settings)?;
        if let Some(checkout) = self.read_meta::<Checkout>("checkout.json")? {
            self.finish_checkout(&store, &checkout)?;
            state = checkout.state;
        }
        f(&mut state, &store)
    }
    fn read_meta<T: serde::de::DeserializeOwned>(&self, name: &str) -> Result<Option<T>> {
        match fs::File::open(self.meta.join(name)) {
            Ok(file) => Ok(Some(serde_json::from_reader(file)?)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn save(&self, state: &State) -> Result<()> {
        atomic_json(&self.meta.join("state.json"), state)
    }
    pub fn configure_storage(&self, settings: Settings) -> Result<()> {
        settings.validate()?;
        self.with_state(|state, _| {
            state.settings = settings;
            self.save(state)
        })
    }
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        ensure!(!paths.is_empty(), "specify paths to stage");
        self.with_state(|state, store| {
            let mut tracked = store.snapshot(state.tip())?;
            tracked.extend(state.index.clone());
            let files = self.scan(&tracked, paths)?;
            state
                .index
                .retain(|p, _| !paths.iter().any(|sel| selected(p, sel)));
            for (path, metadata) in files {
                let mut entry = self.capture(store, &path, &metadata)?;
                if !cfg!(unix)
                    && let Some(previous) = tracked.get(&path)
                {
                    entry.executable = previous.executable;
                }
                state.index.insert(path, entry);
            }
            state
                .conflicts
                .retain(|p| !paths.iter().any(|sel| selected(p, sel)));
            self.save(state)
        })
    }
    pub fn stage_paths(&self, paths: &[PathBuf]) -> Result<()> {
        let cwd = std::env::current_dir()?.canonicalize()?;
        let mut selections = Vec::new();
        for path in paths {
            let absolute = cwd.join(path);
            let mut normalized = PathBuf::new();
            for part in absolute.components() {
                match part {
                    Component::CurDir => (),
                    Component::ParentDir => {
                        normalized.pop();
                    }
                    other => normalized.push(other.as_os_str()),
                }
            }
            let relative = normalized
                .strip_prefix(&self.root)
                .context("path is outside this working copy")?;
            selections.push(if relative.as_os_str().is_empty() {
                ".".into()
            } else {
                portable(relative)?
            });
        }
        self.stage(&selections)
    }
    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        for p in paths {
            if p != "." {
                valid_path(p)?;
            }
        }
        self.with_state(|state, store| {
            ensure!(
                state.merge_parent.is_none(),
                "finish or abort the pending merge first"
            );
            let base = store.snapshot(state.tip())?;
            state
                .index
                .retain(|p, _| !paths.iter().any(|sel| selected(p, sel)));
            state.index.extend(
                base.into_iter()
                    .filter(|(p, _)| paths.iter().any(|sel| selected(p, sel))),
            );
            self.save(state)
        })
    }
    fn capture(&self, store: &Store, path: &str, metadata: &fs::Metadata) -> Result<Entry> {
        let id = store.write_file(fs::File::open(self.safe_path(path)?)?)?;
        let after = self.safe_path(path)?.metadata()?;
        ensure!(
            after.len() == metadata.len() && after.modified()? == metadata.modified()?,
            "{path} changed while staging"
        );
        Ok(Entry {
            id,
            executable: executable(metadata),
        })
    }
    fn safe_path(&self, name: &str) -> Result<PathBuf> {
        valid_path(name)?;
        let mut output = self.root.clone();
        for part in name.split('/') {
            output.push(part);
            safe_metadata(&output)?;
        }
        Ok(output)
    }
    fn scan(
        &self,
        tracked: &Snapshot,
        selections: &[String],
    ) -> Result<BTreeMap<String, fs::Metadata>> {
        let mut ignore = GitignoreBuilder::new(&self.root);
        let ignore_file = self.root.join(".lvcsignore");
        if safe_metadata(&ignore_file)?.is_some_and(|m| m.is_file()) {
            if let Some(error) = ignore.add(ignore_file) {
                return Err(error.into());
            }
        }
        let ignore = ignore.build()?;
        let mut files = BTreeMap::new();
        let mut pending = BTreeSet::new();
        for path in selections {
            pending.insert(if path == "." {
                self.root.clone()
            } else {
                self.safe_path(path)?
            });
        }
        let mut visited = 0;
        while let Some(path) = pending.pop_first() {
            visited += 1;
            ensure!(
                visited <= loki_vcs_core::MAX_FILES * 2,
                "working tree exceeds scan limit"
            );
            let relative = path.strip_prefix(&self.root)?;
            if relative.as_os_str().is_empty() {
                for child in fs::read_dir(&path)? {
                    pending.insert(child?.path());
                }
                continue;
            }
            if relative
                .components()
                .any(|p| p.as_os_str().eq_ignore_ascii_case(".lvcs"))
                || relative
                    .components()
                    .next()
                    .is_some_and(|p| p.as_os_str().to_string_lossy().starts_with(".lvcs-"))
            {
                continue;
            }
            let Some(metadata) = safe_metadata(&path)? else {
                continue;
            };
            let name = portable(relative)?;
            if ignore
                .matched_path_or_any_parents(relative, metadata.is_dir())
                .is_ignore()
                && !tracked.contains_key(&name)
                && !(metadata.is_dir()
                    && tracked
                        .range(format!("{name}/")..)
                        .next()
                        .is_some_and(|(p, _)| selected(p, &name)))
            {
                continue;
            }
            if metadata.is_dir() {
                for child in fs::read_dir(&path)? {
                    pending.insert(child?.path());
                }
            } else if metadata.is_file() {
                files.insert(name, metadata);
            }
        }
        Ok(files)
    }
    pub fn status(&self) -> Result<Status> {
        self.status_with_refresh(false)
    }

    pub fn status_with_refresh(&self, refresh: bool) -> Result<Status> {
        self.with_state(|state, store| self.status_for(state, store, refresh))
    }
    fn status_for(&self, state: &State, store: &Store, refresh: bool) -> Result<Status> {
        let base = store.snapshot(state.tip())?;
        let mut tracked = base.clone();
        tracked.extend(state.index.clone());
        let files = self.scan(&tracked, &[".".into()])?;
        let cache: StatusCache = if refresh {
            BTreeMap::new()
        } else {
            self.read_meta("status.json")
                .ok()
                .flatten()
                .unwrap_or_default()
        };
        let mut next = StatusCache::new();
        let mut unstaged = BTreeMap::new();
        let clock = SystemTime::now();
        for (path, metadata) in &files {
            let Some(entry) = state.index.get(path) else {
                unstaged.insert(path.clone(), "added");
                continue;
            };
            let stamp = Fingerprint::observed(metadata, clock);
            let cached = stamp.as_ref().is_some_and(|stamp| {
                cache.get(path).is_some_and(|(known_entry, known_stamp)| {
                    known_entry == entry && known_stamp == stamp
                })
            });
            if cached || self.matches(store, path, metadata, *entry)? {
                if let Some(stamp) = stamp {
                    next.insert(path.clone(), (*entry, stamp));
                }
            } else {
                unstaged.insert(path.clone(), "modified");
            }
        }
        if refresh || next != cache {
            // The cache is disposable. Failing to write it does not affect the working copy.
            let _ = atomic_json(&self.meta.join("status.json"), &next);
        }
        for path in state.index.keys().filter(|p| !files.contains_key(*p)) {
            unstaged.insert(path.clone(), "deleted");
        }
        Ok(Status {
            branch: match &state.head {
                Head::Branch(name) => Some(name.clone()),
                _ => None,
            },
            revision: state.tip(),
            staged: changes(&base, &state.index),
            unstaged,
            conflicts: state.conflicts.clone(),
        })
    }
    fn matches(
        &self,
        store: &Store,
        path: &str,
        metadata: &fs::Metadata,
        entry: Entry,
    ) -> Result<bool> {
        let manifest = store.manifest(entry.id)?;
        if manifest.size != metadata.len()
            || (cfg!(unix) && executable(metadata) != entry.executable)
        {
            return Ok(false);
        }
        let mut input = fs::File::open(self.safe_path(path)?)?;
        let mut buffer = Vec::new();
        for chunk in manifest.chunks {
            buffer.resize(chunk.size as usize, 0);
            match input.read_exact(&mut buffer) {
                Ok(()) if Id::of(Kind::Chunk, &buffer) == chunk.id => (),
                Ok(()) => return Ok(false),
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(e) => return Err(e.into()),
            }
        }
        Ok(input.read(&mut [0])? == 0)
    }
    pub fn commit(&self, author: &str, message: &str) -> Result<Id> {
        self.with_state(|state, store| self.commit_state(state, store, author, message))
    }
    fn commit_state(
        &self,
        state: &mut State,
        store: &Store,
        author: &str,
        message: &str,
    ) -> Result<Id> {
        ensure!(
            state.conflicts.is_empty(),
            "stage all resolved conflicts before committing"
        );
        let Head::Branch(name) = &state.head else {
            bail!("switch to a branch before committing")
        };
        let parents: Vec<_> = state.tip().into_iter().chain(state.merge_parent).collect();
        let root = store.write_snapshot(&state.index)?;
        ensure!(
            state.merge_parent.is_some()
                || match state.tip() {
                    Some(id) => store.revision(id)?.root != root,
                    None => !state.index.is_empty(),
                },
            "no staged changes"
        );
        let id = store.put(
            Kind::Revision,
            &canonical(&Revision {
                root,
                parents,
                author: author.into(),
                timestamp: now(),
                message: message.into(),
            })?,
        )?;
        state.branches.insert(name.clone(), Some(id));
        state.merge_parent = None;
        self.save(state)?;
        Ok(id)
    }
    pub fn branches(&self) -> Result<BTreeMap<String, Option<Id>>> {
        self.with_state(|state, _| Ok(state.branches.clone()))
    }
    pub fn branch(&self, name: &str, from: Option<&str>) -> Result<()> {
        reference(name)?;
        self.with_state(|state, store| {
            ensure!(!state.branches.contains_key(name), "branch already exists");
            let target = from
                .map(|r| resolve(state, store, r))
                .transpose()?
                .or_else(|| state.tip());
            state.branches.insert(name.into(), target);
            self.save(state)
        })
    }
    pub fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.with_state(|state, store| {
            ensure!(
                !matches!(&state.head, Head::Branch(current) if current == name),
                "cannot delete the checked-out branch"
            );
            let tip = state
                .branches
                .remove(name)
                .context("branch does not exist")?;
            if let Some(tip) = tip.filter(|_| !force) {
                ensure!(
                    match state.tip() {
                        Some(head) => store.is_ancestor(tip, head)?,
                        None => false,
                    },
                    "branch has unmerged revisions"
                );
            }
            self.save(state)
        })
    }
    pub fn tags(&self) -> Result<BTreeMap<String, Id>> {
        self.with_state(|state, _| Ok(state.tags.clone()))
    }
    pub fn tag(&self, name: &str, from: Option<&str>, delete: bool) -> Result<()> {
        reference(name)?;
        self.with_state(|state, store| {
            if delete {
                ensure!(state.tags.remove(name).is_some(), "unknown tag");
            } else {
                ensure!(!state.tags.contains_key(name), "tag already exists");
                let target = from
                    .map(|r| resolve(state, store, r))
                    .transpose()?
                    .or(state.tip())
                    .context("no revision to tag")?;
                state.tags.insert(name.into(), target);
            }
            self.save(state)
        })
    }
    pub fn checkout(&self, target: &str, detach: bool) -> Result<()> {
        self.with_state(|state, store| {
            let mut next = state.clone();
            next.head = if detach {
                Head::Detached(resolve(state, store, target)?)
            } else {
                ensure!(state.branches.contains_key(target), "unknown branch");
                Head::Branch(target.into())
            };
            next.index = store.snapshot(next.tip())?;
            self.transition(state, store, next.index.clone(), next)
        })
    }
    pub fn restore(&self, revision: &str, paths: &[String]) -> Result<()> {
        self.with_state(|state, store| {
            let source = store.snapshot(Some(resolve(state, store, revision)?))?;
            let mut working = state.index.clone();
            for path in paths {
                valid_path(path)?;
                let entry = source
                    .get(path)
                    .context("path does not exist in revision")?;
                let target = self.safe_path(path)?;
                ensure!(
                    safe_metadata(&target)?.is_none_or(|m| m.is_file()),
                    "path is not a regular file: {path}"
                );
                working.insert(path.clone(), *entry);
            }
            // An explicit restore may overwrite the selected files only.
            self.apply_checkout(
                store,
                Checkout {
                    old: state.index.clone(),
                    new: working,
                    state: state.clone(),
                },
                true,
                paths,
            )
        })
    }
    fn transition(
        &self,
        before: &State,
        store: &Store,
        working: Snapshot,
        after: State,
    ) -> Result<()> {
        ensure!(
            before.merge_parent.is_none(),
            "finish or abort the pending merge first"
        );
        ensure!(
            before.index == store.snapshot(before.tip())?,
            "working copy has staged changes"
        );
        self.apply_checkout(
            store,
            Checkout {
                old: before.index.clone(),
                new: working,
                state: after,
            },
            false,
            &[],
        )
    }
    fn apply_checkout(
        &self,
        store: &Store,
        mut journal: Checkout,
        overwrite: bool,
        paths: &[String],
    ) -> Result<()> {
        let changed: BTreeSet<_> = journal
            .old
            .keys()
            .chain(journal.new.keys())
            .filter(|p| journal.old.get(*p) != journal.new.get(*p))
            .cloned()
            .chain(paths.iter().cloned())
            .collect();
        journal.old.retain(|p, _| changed.contains(p));
        journal.new.retain(|p, _| changed.contains(p));
        store.write_snapshot(&journal.state.index)?;
        store.write_snapshot(&journal.new)?;
        for path in &changed {
            let target = self.safe_path(path)?;
            if let Some(metadata) = safe_metadata(&target)? {
                if !overwrite {
                    if let Some(old) = journal.old.get(path) {
                        ensure!(
                            metadata.is_file() && self.matches(store, path, &metadata, *old)?,
                            "local changes would be overwritten: {path}"
                        );
                    } else if metadata.is_dir() {
                        for (file, _) in self.scan(&journal.old, std::slice::from_ref(path))? {
                            ensure!(
                                journal.old.contains_key(&file),
                                "untracked path would be overwritten: {file}"
                            );
                        }
                        // The scan honors ignore rules, so also inspect every directory entry.
                        check_replace_dir(&target, path, &journal.old)?;
                    } else {
                        bail!("untracked path would be overwritten: {path}");
                    }
                }
            } else if journal.old.contains_key(path) && !overwrite {
                bail!("local deletion would be overwritten: {path}");
            }
            let mut ancestor = target.parent();
            while let Some(dir) = ancestor.filter(|dir| *dir != self.root) {
                if safe_metadata(dir)?.is_some_and(|m| m.is_file()) {
                    ensure!(
                        journal
                            .old
                            .contains_key(&portable(dir.strip_prefix(&self.root)?)?),
                        "untracked file blocks {path}"
                    );
                }
                ancestor = dir.parent();
            }
        }
        store.graph(journal.new.values().map(|e| e.id))?;
        atomic_json(&self.meta.join("checkout.json"), &journal)?;
        self.finish_checkout(store, &journal)
    }
    fn finish_checkout(&self, store: &Store, journal: &Checkout) -> Result<()> {
        for path in journal.old.keys().rev() {
            if journal.new.contains_key(path) {
                continue;
            }
            let target = self.safe_path(path)?;
            if safe_metadata(&target)?.is_some_and(|m| m.is_file()) {
                fs::remove_file(&target)?;
            }
            let mut parent = target.parent();
            while let Some(dir) = parent.filter(|dir| *dir != self.root) {
                match fs::remove_dir(dir) {
                    Ok(()) => (),
                    Err(e)
                        if matches!(
                            e.kind(),
                            io::ErrorKind::NotFound
                                | io::ErrorKind::NotADirectory
                                | io::ErrorKind::DirectoryNotEmpty
                        ) =>
                    {
                        break;
                    }
                    Err(e) => return Err(e.into()),
                }
                parent = dir.parent();
            }
        }
        for (path, entry) in &journal.new {
            let target = self.safe_path(path)?;
            if let Some(metadata) = safe_metadata(&target)? {
                if metadata.is_dir() {
                    fs::remove_dir(&target)?;
                }
            }
            let parent = target.parent().context("missing file parent")?;
            fs::create_dir_all(parent)?;
            let mut temp = tempfile::NamedTempFile::new_in(parent)?;
            store.copy_file(entry.id, &mut temp)?;
            set_executable(temp.as_file(), entry.executable)?;
            temp.as_file().sync_all()?;
            temp.persist(target)?;
        }
        let mut directories = BTreeSet::from([self.root.clone()]);
        for path in journal.old.keys().chain(journal.new.keys()) {
            for parent in self
                .safe_path(path)?
                .ancestors()
                .skip(1)
                .take_while(|dir| *dir != self.root)
            {
                if parent.is_dir() {
                    directories.insert(parent.to_owned());
                }
            }
        }
        for dir in directories.iter().rev() {
            sync_directory(dir)?;
        }
        self.save(&journal.state)?;
        fs::remove_file(self.meta.join("checkout.json"))?;
        sync_directory(&self.meta)?;
        Ok(())
    }
    pub fn history(&self, revision: Option<&str>, limit: usize) -> Result<Vec<(Id, Revision)>> {
        self.with_state(|state, store| {
            let root = revision
                .map(|r| resolve(state, store, r))
                .transpose()?
                .or(state.tip());
            store.history(root, limit.min(loki_vcs_core::MAX_GRAPH))
        })
    }
    pub fn diff(
        &self,
        from: Option<&str>,
        to: Option<&str>,
        staged: bool,
    ) -> Result<Vec<Difference>> {
        self.with_state(|state, store| {
            let base = if let Some(from) = from {
                store.snapshot(Some(resolve(state, store, from)?))?
            } else if staged || to.is_some() {
                store.snapshot(state.tip())?
            } else {
                state.index.clone()
            };
            let target = if let Some(to) = to {
                store.snapshot(Some(resolve(state, store, to)?))?
            } else if staged {
                state.index.clone()
            } else {
                let mut target = Snapshot::new();
                for (path, metadata) in self.scan(&state.index, &[".".into()])? {
                    let entry = match state.index.get(&path) {
                        Some(entry) if self.matches(store, &path, &metadata, *entry)? => *entry,
                        _ => self.capture(store, &path, &metadata)?,
                    };
                    target.insert(path, entry);
                }
                target
            };
            changes(&base, &target)
                .into_iter()
                .map(|(path, change)| {
                    let a = file_text(store, base.get(&path).copied())?;
                    let b = file_text(store, target.get(&path).copied())?;
                    let patch = a.zip(b).map(|(a, b)| unified_patch(&path, &a, &b));
                    Ok(Difference {
                        path,
                        change,
                        text: patch,
                    })
                })
                .collect()
        })
    }
    pub fn merge(&self, revision: &str, author: &str, message: &str) -> Result<Merge> {
        self.with_state(|state, store| {
            let theirs = resolve(state, store, revision)?;
            self.merge_tip(state, store, theirs, author, message)
        })
    }
    fn merge_tip(
        &self,
        state: &mut State,
        store: &Store,
        theirs: Id,
        author: &str,
        message: &str,
    ) -> Result<Merge> {
        let Head::Branch(name) = &state.head else {
            bail!("switch to a branch before merging")
        };
        let ours = state.tip();
        let mut next = state.clone();
        if let Some(ours) = ours {
            if store.is_ancestor(theirs, ours)? {
                return Ok(Merge {
                    revision: Some(ours),
                    conflicts: BTreeSet::new(),
                });
            }
        }
        if ours.is_none() || store.is_ancestor(ours.unwrap(), theirs)? {
            next.branches.insert(name.clone(), Some(theirs));
            next.index = store.snapshot(Some(theirs))?;
            self.transition(state, store, next.index.clone(), next)?;
            return Ok(Merge {
                revision: Some(theirs),
                conflicts: BTreeSet::new(),
            });
        }
        let ours = ours.context("missing current revision")?;
        let base = merge_base(store, ours, theirs)?;
        let base = store.snapshot(Some(base))?;
        let left = store.snapshot(Some(ours))?;
        let right = store.snapshot(Some(theirs))?;
        let mut working = Snapshot::new();
        for path in base
            .keys()
            .chain(left.keys())
            .chain(right.keys())
            .collect::<BTreeSet<_>>()
        {
            let (b, a, c) = (
                base.get(path).copied(),
                left.get(path).copied(),
                right.get(path).copied(),
            );
            let entry = if a == c || c == b {
                a
            } else if a == b {
                c
            } else if let (Some(b), Some(a), Some(c)) = (b, a, c) {
                match merge_text(store, b, a, c)? {
                    Some(entry) => Some(entry),
                    None => {
                        next.conflicts.insert(path.clone());
                        Some(a)
                    }
                }
            } else {
                next.conflicts.insert(path.clone());
                a
            };
            if let Some(entry) = entry {
                working.insert(path.clone(), entry);
            }
        }
        next.index = working.clone();
        next.merge_parent = Some(theirs);
        if next.conflicts.is_empty() {
            let root = store.write_snapshot(&next.index).context(
                "merged paths collide; resolve the directory or path naming conflict on one branch",
            )?;
            let id = store.put(
                Kind::Revision,
                &canonical(&Revision {
                    parents: vec![ours, theirs],
                    root,
                    author: author.into(),
                    timestamp: now(),
                    message: message.into(),
                })?,
            )?;
            next.branches.insert(name.clone(), Some(id));
            next.merge_parent = None;
            self.transition(state, store, working, next)?;
            return Ok(Merge {
                revision: Some(id),
                conflicts: BTreeSet::new(),
            });
        }
        let conflicts = next.conflicts.clone();
        self.transition(state, store, working, next)?;
        Ok(Merge {
            revision: None,
            conflicts,
        })
    }
    pub fn abort_merge(&self) -> Result<()> {
        self.with_state(|state, store| {
            ensure!(state.merge_parent.is_some(), "no pending merge");
            let mut next = state.clone();
            next.merge_parent = None;
            next.conflicts.clear();
            next.index = store.snapshot(state.tip())?;
            self.apply_checkout(
                store,
                Checkout {
                    old: state.index.clone(),
                    new: next.index.clone(),
                    state: next,
                },
                false,
                &[],
            )
        })
    }
    pub fn verify(&self) -> Result<()> {
        self.with_state(|state, store| store.verify(roots(state)))
    }
    pub fn gc(&self, grace: Duration) -> Result<usize> {
        self.with_state(|state, store| store.collect(roots(state), grace))
    }
}
fn roots(state: &State) -> Vec<Id> {
    state
        .branches
        .values()
        .copied()
        .flatten()
        .chain(state.tags.values().copied())
        .chain(
            state
                .fetched
                .values()
                .flat_map(|refs| refs.values().copied()),
        )
        .chain(state.index.values().map(|e| e.id))
        .chain(state.tip())
        .chain(state.merge_parent)
        .collect()
}
fn resolve(state: &State, store: &Store, name: &str) -> Result<Id> {
    if name == "HEAD" {
        return state.tip().context("no current revision");
    }
    let id = state
        .branches
        .get(name)
        .copied()
        .flatten()
        .or_else(|| state.tags.get(name).copied())
        .or_else(|| {
            name.strip_prefix("remotes/")
                .and_then(|n| n.split_once('/'))
                .and_then(|(remote, key)| state.fetched.get(remote)?.get(key).copied())
        })
        .or_else(|| name.parse().ok())
        .context("unknown revision")?;
    store.revision(id)?;
    Ok(id)
}
fn selected(path: &str, selection: &str) -> bool {
    selection == "."
        || path == selection
        || path
            .strip_prefix(selection)
            .is_some_and(|rest| rest.starts_with('/'))
}
fn safe_metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(m) => {
            ensure!(
                !m.is_symlink(),
                "symbolic links are unsupported: {}",
                path.display()
            );
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                ensure!(
                    m.file_attributes() & 0x400 == 0,
                    "reparse points are unsupported: {}",
                    path.display()
                );
            }
            ensure!(
                m.is_file() || m.is_dir(),
                "unsupported file type: {}",
                path.display()
            );
            Ok(Some(m))
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
fn portable(path: &Path) -> Result<String> {
    let mut names = Vec::new();
    for part in path.components() {
        let Component::Normal(name) = part else {
            bail!("invalid path component")
        };
        names.push(name.to_str().context("path is not UTF-8")?);
    }
    let name = names.join("/");
    valid_path(&name)?;
    Ok(name)
}
fn check_replace_dir(dir: &Path, prefix: &str, old: &Snapshot) -> Result<()> {
    for child in fs::read_dir(dir)? {
        let child = child?;
        let name = child.file_name();
        let key = format!(
            "{prefix}/{}",
            name.to_str().context("non-UTF-8 path blocks checkout")?
        );
        let m = safe_metadata(&child.path())?.context("directory changed during checkout")?;
        ensure!(
            key.split('/').count() <= loki_vcs_core::MAX_DEPTH,
            "directory exceeds checkout depth"
        );
        if m.is_dir() {
            ensure!(
                old.range(format!("{key}/")..)
                    .next()
                    .is_some_and(|(p, _)| selected(p, &key)),
                "untracked directory blocks checkout: {key}"
            );
            check_replace_dir(&child.path(), &key, old)?;
        } else {
            ensure!(
                old.contains_key(&key),
                "untracked file blocks checkout: {key}"
            );
        }
    }
    Ok(())
}
#[cfg(unix)]
fn executable(m: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o111 != 0
}
#[cfg(not(unix))]
fn executable(_: &fs::Metadata) -> bool {
    false
}
fn set_executable(file: &fs::File, executable: bool) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(if executable {
            0o755
        } else {
            0o644
        }))?;
    }
    #[cfg(not(unix))]
    let _ = (file, executable);
    Ok(())
}
fn changes(base: &Snapshot, target: &Snapshot) -> BTreeMap<String, &'static str> {
    base.keys()
        .chain(target.keys())
        .filter_map(|path| {
            let change = match (base.get(path), target.get(path)) {
                (Some(a), Some(b)) if a == b => return None,
                (Some(_), Some(_)) => "modified",
                (None, Some(_)) => "added",
                _ => "deleted",
            };
            Some((path.clone(), change))
        })
        .collect()
}
fn merge_base(store: &Store, ours: Id, theirs: Id) -> Result<Id> {
    let ours = store.history([ours], loki_vcs_core::MAX_GRAPH)?;
    let theirs = store.history([theirs], loki_vcs_core::MAX_GRAPH)?;
    let ours: BTreeSet<_> = ours.into_iter().map(|(id, _)| id).collect();
    let mut candidates = BTreeSet::new();
    let mut ancestors = BTreeSet::new();
    for (id, revision) in theirs {
        if ours.contains(&id) {
            candidates.insert(id);
            ancestors.extend(revision.parents);
        }
    }
    let mut bases = candidates.difference(&ancestors);
    let base = *bases.next().context("histories have no common ancestor")?;
    ensure!(
        bases.next().is_none(),
        "multiple merge bases require an explicit reconciliation"
    );
    Ok(base)
}
fn merge_text(store: &Store, base: Entry, ours: Entry, theirs: Entry) -> Result<Option<Entry>> {
    let (Some(base_text), Some(ours_text), Some(theirs_text)) = (
        file_text(store, Some(base))?,
        file_text(store, Some(ours))?,
        file_text(store, Some(theirs))?,
    ) else {
        return Ok(None);
    };
    let mut edits = Vec::new();
    for version in [&ours_text, &theirs_text] {
        let diff = TextDiff::configure()
            .timeout(Duration::from_millis(150))
            .diff_lines(&base_text, version);
        for op in diff
            .ops()
            .iter()
            .filter(|op| op.tag() != similar::DiffTag::Equal)
        {
            let range = op.old_range();
            let replacement = op
                .new_range()
                .filter_map(|i| diff.new_slice(i))
                .collect::<String>();
            edits.push((range.start, range.end, replacement));
        }
    }
    edits.sort();
    edits.dedup();
    let original = TextDiff::from_lines(&base_text, &base_text);
    let original: Vec<_> = (0..).map_while(|i| original.old_slice(i)).collect();
    let mut merged = String::new();
    let mut cursor = 0;
    let mut previous = None;
    for (start, end, replacement) in edits {
        if start < cursor || previous == Some(start) {
            return Ok(None);
        }
        ensure!(
            start <= end && end <= original.len(),
            "unsupported line boundaries"
        );
        merged.push_str(&original[cursor..start].concat());
        merged.push_str(&replacement);
        cursor = end;
        previous = Some(start);
    }
    merged.push_str(&original[cursor..].concat());
    let executable = if ours.executable == base.executable {
        theirs.executable
    } else {
        ours.executable
    };
    Ok(Some(Entry {
        id: store.write_file(merged.as_bytes())?,
        executable,
    }))
}
