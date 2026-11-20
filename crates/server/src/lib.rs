use anyhow::{Context, Result, ensure};
use loki_vcs_core::{
    Id, Kind, Settings, Store, VERSION, atomic_json, disk_lock, now,
    protocol::{Lock, MAX_NEGOTIATION, MAX_REFS, Refs, Update},
    reference, valid_path,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Debug)]
pub struct Conflict;
impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("reference or lock changed")
    }
}
impl std::error::Error for Conflict {}

#[derive(Serialize, Deserialize)]
struct State {
    refs: Refs,
    locks: BTreeMap<String, Lock>,
}

#[derive(Clone)]
pub struct Hosted {
    root: PathBuf,
    store: Store,
}

impl Hosted {
    pub fn create(root: &Path, default_branch: &str) -> Result<Self> {
        reference(default_branch)?;
        fs::create_dir(root)?;
        let store = Store::create(&root.join("objects"), Settings::default())?;
        let host = Self {
            root: root.to_owned(),
            store,
        };
        host.save(&State {
            refs: Refs {
                version: VERSION,
                default_branch: default_branch.into(),
                branches: BTreeMap::new(),
                tags: BTreeMap::new(),
            },
            locks: BTreeMap::new(),
        })?;
        Ok(host)
    }

    /// Create an independent copy of the complete advertised history.
    /// The source and destination must be hosted in an exclusive administration operation.
    pub fn fork(
        &self,
        root: &Path,
        default: Option<&str>,
        mut reserve: impl FnMut(Id, u64) -> Result<()>,
    ) -> Result<Self> {
        let _source = disk_lock(&self.root.join("writer.lock"))?;
        let mut refs = self.refs()?;
        if let Some(name) = default {
            reference(name)?;
            ensure!(
                name == refs.default_branch || refs.branches.contains_key(name),
                "default branch must exist"
            );
            refs.default_branch = name.into();
        }
        let ids = self
            .store
            .graph(refs.branches.values().chain(refs.tags.values()).copied())?;
        let target = Self::create(root, &refs.default_branch)?;
        let _target = disk_lock(&target.root.join("writer.lock"))?;
        for id in ids {
            let raw = self.store.read(id)?;
            reserve(id, raw.len() as u64 + 1)?;
            target.store.receive(id, &raw)?;
        }
        target.save(&State {
            refs,
            locks: BTreeMap::new(),
        })?;
        Ok(target)
    }

    pub fn open(root: &Path) -> Result<Self> {
        let host = Self {
            root: root.to_owned(),
            store: Store::open(&root.join("objects"), Settings::default())?,
        };
        ensure!(
            host.root.join("refs.json").is_file(),
            "missing repository reference metadata"
        );
        Ok(host)
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    fn state(&self) -> Result<State> {
        let state: State = serde_json::from_reader(fs::File::open(self.root.join("refs.json"))?)?;
        ensure!(
            state.refs.version == VERSION,
            "unsupported repository version"
        );
        Ok(state)
    }

    fn save(&self, state: &State) -> Result<()> {
        atomic_json(&self.root.join("refs.json"), state)
    }

    pub fn refs(&self) -> Result<Refs> {
        Ok(self.state()?.refs)
    }

    pub fn set_default_branch(&self, name: &str) -> Result<()> {
        reference(name)?;
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        let mut state = self.state()?;
        if state.refs.default_branch != name {
            ensure!(
                state.refs.branches.contains_key(name),
                "default branch must exist"
            );
            state.refs.default_branch = name.into();
            self.save(&state)?;
        }
        Ok(())
    }

    pub fn missing(&self, ids: Vec<Id>) -> Result<Vec<Id>> {
        ensure!(
            ids.len() <= MAX_NEGOTIATION,
            "too many objects in negotiation"
        );
        Ok(ids
            .into_iter()
            .filter(|id| !self.store.contains(*id))
            .collect())
    }

    /// Authorization and quota hooks belong to the embedding service. Quota
    /// reservation runs inside the writer lock before a new object is stored.
    pub fn receive(
        &self,
        id: Id,
        raw: &[u8],
        reserve: impl FnOnce(u64) -> Result<()>,
    ) -> Result<()> {
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        if !self.store.contains(id) {
            reserve(raw.len() as u64 + 1)?;
        }
        self.store.receive(id, raw)
    }

    pub fn update(&self, tags: bool, name: &str, change: Update) -> Result<()> {
        reference(name)?;
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        let mut state = self.state()?;
        let count = state.refs.branches.len() + state.refs.tags.len();
        let refs = if tags {
            &mut state.refs.tags
        } else {
            &mut state.refs.branches
        };
        let old = refs.get(name).copied();
        if old != change.expected {
            return Err(Conflict.into());
        }
        if let Some(target) = change.target {
            ensure!(target.kind() == Kind::Revision, "target must be a revision");
            if old == Some(target) {
                return Ok(());
            }
            ensure!(old.is_some() || count < MAX_REFS, "reference limit reached");
            if let Some(old) = old {
                ensure!(
                    !tags && self.store.is_ancestor(old, target)?,
                    "reference update must fast-forward"
                );
            }
            self.store
                .validate_history(target, &old.into_iter().collect::<HashSet<_>>())?;
            refs.insert(name.into(), target);
        } else {
            ensure!(
                tags || name != state.refs.default_branch,
                "cannot delete default branch"
            );
            refs.remove(name);
        }
        self.save(&state)
    }

    pub fn locks(&self) -> Result<BTreeMap<String, Lock>> {
        let mut locks = self.state()?.locks;
        locks.retain(|_, lock| lock.expires > now());
        Ok(locks)
    }

    pub fn lock(&self, path: &str, owner: &str, seconds: Option<u32>) -> Result<Option<Lock>> {
        valid_path(path)?;
        ensure!(
            !owner.is_empty() && owner.len() <= 256,
            "invalid lock owner"
        );
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        let mut state = self.state()?;
        let time = now();
        state.locks.retain(|_, lock| lock.expires > time);
        let key = path.to_uppercase();
        if state
            .locks
            .get(&key)
            .is_some_and(|lock| lock.owner != owner)
        {
            return Err(Conflict.into());
        }
        let result = if let Some(seconds) = seconds {
            ensure!(
                (60..=86_400).contains(&seconds),
                "lock lifetime must be between 60 seconds and 24 hours"
            );
            ensure!(
                state.locks.contains_key(&key) || state.locks.len() < 4096,
                "lock limit reached"
            );
            let lock = Lock {
                owner: owner.into(),
                expires: time + i64::from(seconds),
            };
            state.locks.insert(key, lock.clone());
            Some(lock)
        } else {
            state.locks.remove(&key);
            None
        };
        self.save(&state)?;
        Ok(result)
    }

    fn roots(&self) -> Result<Vec<Id>> {
        let state = self.refs()?;
        Ok(state
            .branches
            .into_values()
            .chain(state.tags.into_values())
            .collect())
    }

    pub fn verify(&self) -> Result<()> {
        self.verify_with([])
    }

    pub fn verify_with(&self, extra: impl IntoIterator<Item = Id>) -> Result<()> {
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        self.store.verify(self.roots()?.into_iter().chain(extra))
    }

    pub fn gc(&self, grace: Duration) -> Result<usize> {
        self.gc_with(grace, [])
    }

    pub fn gc_with(&self, grace: Duration, extra: impl IntoIterator<Item = Id>) -> Result<usize> {
        let _lock = disk_lock(&self.root.join("writer.lock"))?;
        self.store.collect(
            self.roots()?.into_iter().chain(extra),
            grace.max(Duration::from_secs(86_400)),
        )
    }

    pub fn entry(&self, revision: Id, path: &str) -> Result<loki_vcs_core::Entry> {
        valid_path(path)?;
        let mut entry = loki_vcs_core::Entry {
            id: self.store.revision(revision)?.root,
            executable: false,
        };
        for name in path.split('/') {
            entry = *self
                .store
                .tree(entry.id)?
                .get(name)
                .context("path does not exist")?;
        }
        Ok(entry)
    }
}
