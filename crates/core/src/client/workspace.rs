use super::{Checkout, Remote, Repository, State, network, resolve, roots, selected};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use crate::{Entry, Id, Kind, Store, text, valid_path};
use std::{
    collections::{BTreeMap, HashSet},
    time::Duration,
};

/// Most specific root-relative path wins. A rule covers the path and its descendants.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceSelection(BTreeMap<String, bool>);

impl WorkspaceSelection {
    pub(super) fn from_paths(paths: &[String]) -> Result<Self> {
        let mut selection = Self::default();
        for path in paths {
            selection.set_rule(&rule_path(path)?, true);
        }
        Ok(selection)
    }

    pub fn rules(&self) -> &BTreeMap<String, bool> {
        &self.0
    }

    pub(super) fn includes(&self, mut path: &str) -> bool {
        loop {
            if let Some(include) = self.0.get(path) {
                return *include;
            }
            if let Some((parent, _)) = path.rsplit_once('/') {
                path = parent;
            } else {
                return self.0.get(".") == Some(&true);
            }
        }
    }

    pub(super) fn may_include(&self, path: &str) -> bool {
        let prefix = format!("{path}/");
        self.includes(path)
            || self
                .0
                .range(prefix.clone()..)
                .take_while(|(rule, _)| rule.starts_with(&prefix))
                .any(|(_, include)| *include)
    }

    pub(super) fn set_rule(&mut self, path: &str, include: bool) {
        self.0.retain(|rule, _| !selected(rule, path));
        self.0.insert(path.into(), include);
    }

    fn changed(mut self, change: &WorkspaceChange) -> Result<Self> {
        let (paths, include) = match change {
            WorkspaceChange::Set(paths) => return Self::from_paths(paths),
            WorkspaceChange::Add(paths) => (paths, true),
            WorkspaceChange::Remove(paths) => (paths, false),
        };
        for path in paths {
            self.set_rule(&rule_path(path)?, include);
        }
        Ok(self)
    }
}

fn rule_path(value: &str) -> Result<String> {
    let normalized = value.replace('\\', "/");
    let normalized = normalized.trim_end_matches('/');
    let path = normalized.strip_prefix("./").unwrap_or(normalized);
    if path != "." {
        valid_path(path)?;
    }
    Ok(path.into())
}

#[derive(Debug)]
pub struct WorkspaceFile {
    pub path: String,
    pub size: u64,
    pub included: bool,
}

pub enum WorkspaceChange {
    Set(Vec<String>),
    Add(Vec<String>),
    Remove(Vec<String>),
}

#[derive(Debug)]
pub struct WorkspacePlan {
    pub add: Vec<String>,
    pub remove: Vec<String>,
}

impl State {
    pub(super) fn content_remote(&self, token: Option<&str>) -> Result<Option<Remote>> {
        match token {
            Some(token) if self.sparse.is_some() && self.remotes.contains_key("origin") => {
                Ok(Some(network::configured(self, "origin", token)?))
            }
            _ => Ok(None),
        }
    }

    fn selection(&self) -> WorkspaceSelection {
        self.sparse
            .clone()
            .unwrap_or_else(|| WorkspaceSelection(BTreeMap::from([(".".into(), true)])))
    }
}

impl Repository {
    pub fn workspace_selection(&self) -> Result<WorkspaceSelection> {
        self.with_state(|state, _| Ok(state.selection()))
    }

    pub fn workspace_files(&self, revision: Option<&str>) -> Result<Vec<WorkspaceFile>> {
        self.with_state(|state, store| {
            let snapshot = match revision {
                Some(revision) => store.snapshot(Some(resolve(state, store, revision)?))?,
                None => state.index.clone(),
            };
            snapshot
                .into_iter()
                .map(|(path, entry)| {
                    Ok(WorkspaceFile {
                        size: store.manifest(entry.id)?.size,
                        included: state.includes(&path),
                        path,
                    })
                })
                .collect()
        })
    }

    /// Preview or apply a selection without changing repository refs or the full index.
    pub fn change_workspace(
        &self,
        change: &WorkspaceChange,
        preview: bool,
        token: Option<&str>,
    ) -> Result<WorkspacePlan> {
        self.with_state(|state, store| {
            let old_selection = state.selection();
            let selection = old_selection.clone().changed(change)?;
            let old = state.project(state.index.clone());
            let mut new = state.index.clone();
            new.retain(|path, _| selection.includes(path));
            let base = store.snapshot(state.tip())?;
            let plan = WorkspacePlan {
                add: new
                    .keys()
                    .filter(|p| !old.contains_key(*p))
                    .cloned()
                    .collect(),
                remove: old
                    .keys()
                    .filter(|p| !new.contains_key(*p))
                    .cloned()
                    .collect(),
            };
            if preview || old_selection == selection {
                return Ok(plan);
            }
            ensure!(
                state.merge_parent.is_none(),
                "finish or abort the pending merge first"
            );
            for path in base.keys().chain(state.index.keys()) {
                if state.includes(path) && !selection.includes(path) {
                    ensure!(
                        base.get(path) == state.index.get(path),
                        "staged change blocks removal from the workspace: {path}"
                    );
                }
            }
            let reduces = old_selection
                .rules()
                .keys()
                .chain(selection.rules().keys())
                .any(|path| old_selection.includes(path) && !selection.includes(path));
            if reduces {
                // Include ignored files in the safety check so shrinking a selection
                // cannot silently hide untracked work.
                for path in self
                    .scan(&old, &[".".into()], state.sparse.as_ref(), false)?
                    .keys()
                {
                    ensure!(
                        selection.includes(path) || old.contains_key(path),
                        "untracked file blocks removal from the workspace: {path}"
                    );
                }
            }
            let mut next = state.clone();
            next.sparse = Some(selection);
            let remote = next.content_remote(token)?;
            self.apply_checkout(
                store,
                Checkout {
                    old,
                    new,
                    state: next,
                },
                false,
                &[],
                remote.as_ref(),
            )?;
            Ok(plan)
        })
    }

    /// Evict chunks that this workspace can re-download from origin.
    pub fn prune_workspace(&self, token: &str) -> Result<usize> {
        self.with_state(|state, store| {
            ensure!(state.sparse.is_some(), "configure a sparse workspace first");
            ensure!(
                state.merge_parent.is_none(),
                "finish or abort the pending merge first"
            );
            let remote = network::configured(state, "origin", token)?;
            let mut next = state.clone();
            let refs = network::fetch(&mut next, store, &remote, "origin")?;
            let remote_objects = store.metadata_graph(
                refs.branches.values().chain(refs.tags.values()).copied(),
                &HashSet::new(),
            )?;
            let remote_revisions = remote_objects
                .iter()
                .copied()
                .filter(|id| id.kind() == Kind::Revision)
                .collect();
            let revision_roots = roots(state)
                .into_iter()
                .filter(|id| id.kind() == Kind::Revision);
            let mut keep = store.metadata_graph(revision_roots, &remote_revisions)?;
            let head = state.project(store.snapshot(state.tip())?);
            let index = state.project(state.index.clone());
            keep.extend(store.graph(head.values().chain(index.values()).map(|entry| entry.id))?);
            keep.extend(
                store
                    .object_ids()?
                    .into_iter()
                    .filter(|id| id.kind() != Kind::Chunk || !remote_objects.contains(id)),
            );
            // Leave cache entries that origin no longer advertises alone. They may
            // be the only local copy of objects from an abandoned branch.
            store.collect_except(&keep, Duration::ZERO)
        })
    }

    pub(super) fn verify_sparse(&self, state: &State, store: &Store) -> Result<()> {
        let selected = state.project(state.index.clone());
        store.graph(selected.values().map(|entry| entry.id))?;
        store.verify_cached(roots(state))
    }
}

pub(super) fn hydrate(
    store: &Store,
    files: impl IntoIterator<Item = Id>,
    remote: Option<&Remote>,
) -> Result<()> {
    let ids = store.metadata_graph(files, &HashSet::new())?;
    for id in ids
        .into_iter()
        .filter(|id| id.kind() == Kind::Chunk && !store.contains(*id))
    {
        let remote = remote.with_context(|| {
            format!("content {id} is not cached; connect to origin to download it")
        })?;
        remote
            .download(store, id)
            .with_context(|| format!("could not download {id} from origin"))?;
    }
    Ok(())
}

pub(super) fn cached_text(
    store: &Store,
    entry: Option<Entry>,
    sparse: bool,
) -> Result<Option<String>> {
    if sparse && let Some(entry) = entry {
        let manifest = store.manifest(entry.id)?;
        if manifest.size > text::TEXT_LIMIT as u64
            || manifest
                .chunks
                .iter()
                .any(|chunk| !store.contains(chunk.id))
        {
            return Ok(None);
        }
    }
    text::file_text(store, entry)
}
