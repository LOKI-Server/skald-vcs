use super::{Head, Merge, Repository, State, WorkspaceSelection, resolve};
use anyhow::{Context, Result, ensure};
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder, Response},
};
use crate::{
    Id, Kind, MAX_GRAPH, MAX_OBJECT, Object, Settings, Store, VERSION,
    protocol::{MAX_NEGOTIATION, MAX_REFS, Objects, Refs, Update},
    read_bounded, reference,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::Path,
    time::Duration,
};

pub struct Remote {
    base: Url,
    token: String,
    http: Client,
}

impl Remote {
    pub fn new(url: &str, token: impl Into<String>) -> Result<Self> {
        Ok(Self {
            base: validate_url(url)?,
            token: token.into(),
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(90))
                .build()?,
        })
    }

    fn url(&self, path: &[&str]) -> Result<Url> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("invalid remote URL"))?
            .pop_if_empty()
            .extend(path);
        Ok(url)
    }

    fn request(&self, request: RequestBuilder) -> Result<Response> {
        let request = if self.token.is_empty() {
            request
        } else {
            request.bearer_auth(&self.token)
        };
        let response = request.send()?;
        let status = response.status();
        if !status.is_success() {
            match refusal(response) {
                Some(reason) => anyhow::bail!("remote refused the request ({status}): {reason}"),
                None => anyhow::bail!("remote returned {status}"),
            }
        }
        Ok(response)
    }

    pub fn refs(&self) -> Result<Refs> {
        let response = self.request(self.http.get(self.url(&["refs"])?))?;
        let refs: Refs = serde_json::from_slice(&read_bounded(response, 2 * 1024 * 1024)?)?;
        ensure!(
            refs.version == VERSION,
            "incompatible remote protocol version"
        );
        reference(&refs.default_branch)?;
        ensure!(
            refs.branches.len() + refs.tags.len() <= MAX_REFS,
            "too many references"
        );
        for (name, id) in refs.branches.iter().chain(&refs.tags) {
            reference(name)?;
            ensure!(id.kind() == Kind::Revision, "invalid remote reference");
        }
        Ok(refs)
    }

    pub(super) fn download(&self, store: &Store, id: Id) -> Result<Object> {
        let response = self.request(self.http.get(self.url(&["objects", &id.to_string()])?))?;
        let raw = read_bounded(response, MAX_OBJECT)?;
        store.receive(id, &raw)?;
        Object::parse(id, &raw)
    }

    fn receive(
        &self,
        store: &Store,
        refs: &Refs,
        trusted: &HashSet<Id>,
        sparse: bool,
    ) -> Result<()> {
        let mut seen = HashSet::new();
        let mut pending: Vec<_> = refs
            .branches
            .values()
            .chain(refs.tags.values())
            .copied()
            .collect();
        while let Some(id) = pending.pop() {
            if (sparse && id.kind() == Kind::Chunk) || trusted.contains(&id) || !seen.insert(id) {
                continue;
            }
            ensure!(
                seen.len() <= MAX_GRAPH,
                "remote history exceeds graph limit"
            );
            let object = if store.contains(id) {
                store.object(id)?
            } else {
                self.download(store, id)?
            };
            if !sparse || !matches!(object, Object::File(_)) {
                pending.extend(object.children());
            }
        }
        let tips = refs.branches.values().chain(refs.tags.values()).copied();
        if sparse {
            store.metadata_graph(tips, trusted)?;
        } else {
            store.graph_since(tips, trusted)?;
        }
        Ok(())
    }

    fn send(&self, store: &Store, tip: Id, trusted: &HashSet<Id>, sparse: bool) -> Result<()> {
        let ids = if sparse {
            store.metadata_graph([tip], trusted)?
        } else {
            store.graph_since([tip], trusted)?
        };
        let ids: Vec<_> = ids.into_iter().collect();
        for batch in ids.chunks(MAX_NEGOTIATION) {
            let response =
                self.request(self.http.post(self.url(&["missing"])?).json(&Objects {
                    ids: batch.to_vec(),
                }))?;
            let missing: Objects =
                serde_json::from_slice(&read_bounded(response, MAX_NEGOTIATION * 70)?)?;
            let mut unique = HashSet::new();
            for id in missing.ids {
                ensure!(
                    batch.contains(&id) && unique.insert(id),
                    "invalid remote negotiation response"
                );
                self.request(
                    self.http
                        .put(self.url(&["objects", &id.to_string()])?)
                        .header("content-type", "application/octet-stream")
                        .body(store.read(id).with_context(|| {
                            format!(
                                "destination needs object {id}; download its content before pushing"
                            )
                        })?),
                )?;
            }
        }
        Ok(())
    }

    pub fn locks(&self) -> Result<serde_json::Value> {
        let response = self.request(self.http.get(self.url(&["locks"])?))?;
        Ok(serde_json::from_slice(&read_bounded(
            response,
            8 * 1024 * 1024,
        )?)?)
    }

    pub fn lock(
        &self,
        path: &str,
        seconds: Option<u32>,
    ) -> Result<Option<crate::protocol::Lock>> {
        crate::valid_path(path)?;
        let response = self.request(
            self.http
                .put(self.url(&["locks"])?)
                .json(&serde_json::json!({"path":path,"seconds":seconds})),
        )?;
        #[derive(serde::Deserialize)]
        struct Locked {
            lock: Option<crate::protocol::Lock>,
        }
        let result: Locked = serde_json::from_slice(&read_bounded(response, 4096)?)?;
        Ok(result.lock)
    }

    pub fn update(&self, tags: bool, name: &str, change: &Update) -> Result<()> {
        reference(name)?;
        let kind = if tags { "tags" } else { "branches" };
        self.request(self.http.put(self.url(&["refs", kind, name])?).json(change))?;
        Ok(())
    }
}

/// The reason a server gave for refusing a request, from a JSON `error` field.
/// The text comes from the remote, so control characters are dropped and its
/// length is capped before it reaches a terminal or dialog.
fn refusal(response: Response) -> Option<String> {
    let body = read_bounded(response, 4096).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let reason: String = value
        .get("error")?
        .as_str()?
        .chars()
        .filter(|c| !c.is_control())
        .take(300)
        .collect();
    let reason = reason.trim();
    (!reason.is_empty()).then(|| reason.to_owned())
}

fn validate_url(url: &str) -> Result<Url> {
    let mut url = Url::parse(url)?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "remote requires HTTPS"
    );
    ensure!(
        url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "invalid remote URL"
    );
    url.set_path(url.path().trim_end_matches('/').to_owned().as_str());
    Ok(url)
}

pub(super) fn fetch(state: &mut State, store: &Store, remote: &Remote, name: &str) -> Result<Refs> {
    let refs = remote.refs()?;
    let trusted = state
        .fetched
        .get(name)
        .into_iter()
        .flat_map(|refs| refs.values().copied())
        .collect();
    ensure!(
        state.sparse.is_none() || name == "origin",
        "sparse fetch requires origin"
    );
    remote.receive(store, &refs, &trusted, state.sparse.is_some())?;
    let fetched = refs
        .branches
        .iter()
        .map(|(name, id)| (format!("branches/{name}"), *id))
        .chain(
            refs.tags
                .iter()
                .map(|(name, id)| (format!("tags/{name}"), *id)),
        )
        .collect();
    state.fetched.insert(name.into(), fetched);
    Ok(refs)
}

pub(super) fn configured(state: &State, name: &str, token: &str) -> Result<Remote> {
    Remote::new(state.remotes.get(name).context("unknown remote")?, token)
}

impl Repository {
    pub fn clone_remote(destination: &Path, url: &str, token: &str) -> Result<Self> {
        Self::clone_with_selection(destination, url, token, None)
    }

    pub fn clone_sparse(
        destination: &Path,
        url: &str,
        token: &str,
        paths: &[String],
    ) -> Result<Self> {
        let selection = WorkspaceSelection::from_paths(paths)?;
        Self::clone_with_selection(destination, url, token, Some(selection))
    }

    fn clone_with_selection(
        destination: &Path,
        url: &str,
        token: &str,
        sparse: Option<WorkspaceSelection>,
    ) -> Result<Self> {
        ensure!(!destination.exists(), "clone destination already exists");
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = tempfile::Builder::new()
            .prefix(".skald-clone-")
            .tempdir_in(parent)?;
        let linked = Self::init(temp.path(), Settings::default())?;
        let remote = Remote::new(url, token)?;
        linked.with_state(|state, store| {
            let mut next = state.clone();
            next.remotes.insert("origin".into(), url.into());
            next.sparse = sparse;
            let refs = fetch(&mut next, store, &remote, "origin")?;
            next.branches = refs
                .branches
                .into_iter()
                .map(|(name, id)| (name, Some(id)))
                .collect();
            next.branches
                .entry(refs.default_branch.clone())
                .or_insert(None);
            next.tags = refs.tags;
            next.head = Head::Branch(refs.default_branch);
            next.index = store.snapshot(next.tip())?;
            linked.transition(state, store, next.index.clone(), next, Some(&remote))
        })?;
        fs::rename(temp.path(), destination)?;
        crate::sync_directory(parent)?;
        Self::open(destination)
    }

    pub fn remotes(&self) -> Result<BTreeMap<String, String>> {
        self.with_state(|state, _| Ok(state.remotes.clone()))
    }

    pub fn remote(&self, name: &str, url: Option<&str>) -> Result<()> {
        reference(name)?;
        ensure!(!name.contains('/'), "remote names cannot contain slashes");
        self.with_state(|state, _| {
            if state.sparse.is_some() && name == "origin" {
                ensure!(
                    state.remotes.get(name).map(String::as_str) == url,
                    "origin supplies this sparse workspace; keep it configured"
                );
            }
            if let Some(url) = url {
                validate_url(url)?;
                if state.remotes.get(name).is_some_and(|old| old != url) {
                    state.fetched.remove(name);
                }
                state.remotes.insert(name.into(), url.into());
            } else {
                ensure!(state.remotes.remove(name).is_some(), "unknown remote");
                state.fetched.remove(name);
            }
            self.save(state)
        })
    }

    pub fn fetch(&self, name: &str, token: &str) -> Result<Refs> {
        self.with_state(|state, store| {
            let remote = configured(state, name, token)?;
            let refs = fetch(state, store, &remote, name)?;
            self.save(state)?;
            Ok(refs)
        })
    }

    pub fn push(
        &self,
        name: &str,
        token: &str,
        reference_name: Option<&str>,
        tags: bool,
        delete: bool,
    ) -> Result<()> {
        self.with_state(|state, store| {
            let remote = configured(state, name, token)?;
            let branch = match reference_name {
                Some(name) => name,
                None if !tags => match &state.head {
                    Head::Branch(name) => name,
                    _ => anyhow::bail!("specify a branch"),
                },
                None => anyhow::bail!("specify a tag"),
            };
            reference(branch)?;
            let refs = remote.refs()?;
            let expected = if tags { &refs.tags } else { &refs.branches }
                .get(branch)
                .copied();
            let target = if delete {
                None
            } else if tags {
                Some(*state.tags.get(branch).context("unknown tag")?)
            } else {
                Some(
                    state
                        .branches
                        .get(branch)
                        .copied()
                        .flatten()
                        .context("branch has no revision")?,
                )
            };
            if let (Some(expected), Some(target)) = (expected, target) {
                ensure!(
                    expected == target
                        || (!tags
                            && store.contains(expected)
                            && store.is_ancestor(expected, target)?),
                    "remote has diverged; fetch and reconcile before pushing"
                );
            }
            if let Some(target) = target.filter(|target| Some(*target) != expected) {
                let trusted = refs
                    .branches
                    .values()
                    .chain(refs.tags.values())
                    .copied()
                    .collect();
                remote.send(store, target, &trusted, state.sparse.is_some())?;
            }
            if expected != target {
                remote.update(tags, branch, &Update { expected, target })?;
            }
            let key = format!("{}/{branch}", if tags { "tags" } else { "branches" });
            let fetched = state.fetched.entry(name.into()).or_default();
            if let Some(target) = target {
                fetched.insert(key, target);
            } else {
                fetched.remove(&key);
            }
            self.save(state)
        })
    }

    pub fn pull(
        &self,
        name: &str,
        token: &str,
        branch: Option<&str>,
        author: &str,
    ) -> Result<Merge> {
        self.with_state(|state, store| {
            let remote = configured(state, name, token)?;
            let refs = fetch(state, store, &remote, name)?;
            self.save(state)?;
            let branch = match branch {
                Some(branch) => branch,
                None => match &state.head {
                    Head::Branch(name) => name,
                    _ => &refs.default_branch,
                },
            };
            let target = resolve(state, store, &format!("remotes/{name}/branches/{branch}"))?;
            self.merge_tip(
                state,
                store,
                target,
                author,
                &format!("Merge {name}/{branch}"),
                Some(&remote),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Remote;
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    /// Answers one request with `status` and `body`, and returns the remote URL.
    fn answer_once(status: &'static str, body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/skald/v1/repos/test",
            listener.local_addr().unwrap()
        );
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let reply = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).unwrap();
        });
        url
    }

    #[test]
    fn refusals_carry_the_servers_reason_without_control_characters() {
        let url = answer_once(
            "423 Locked",
            r#"{"error":"Changes are paused \u001b[31muntil the transfer is decided."}"#,
        );
        let error = Remote::new(&url, "token")
            .unwrap()
            .refs()
            .unwrap_err()
            .to_string();
        assert!(error.contains("423 Locked"), "{error}");
        assert!(
            error.contains("Changes are paused [31muntil the transfer is decided."),
            "{error}"
        );
        assert!(!error.contains('\u{1b}'), "{error}");
    }

    #[test]
    fn refusals_without_a_reason_keep_the_status() {
        let url = answer_once("502 Bad Gateway", "<html>proxy</html>");
        let error = Remote::new(&url, "")
            .unwrap()
            .refs()
            .unwrap_err()
            .to_string();
        assert_eq!(error, "remote returned 502 Bad Gateway");
    }
}
