use loki_vcs_client::Repository;
use loki_vcs_core::Settings;
use std::{fs, time::Duration};

fn add(repo: &Repository, path: &str, content: &[u8], title: &str) -> String {
    let file = repo.root().join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, content).unwrap();
    repo.stage(&[path.into()]).unwrap();
    repo.commit("Developer", title).unwrap().to_string()
}

#[test]
fn history_merge_restore_and_ignore_work_without_network() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), Settings::default()).unwrap();
    let base = add(&repo, "doc.txt", b"top\nmiddle\nbottom\n", "initial");
    repo.branch("feature", None).unwrap();
    let left = add(&repo, "doc.txt", b"LEFT\nmiddle\nbottom\n", "change top");
    repo.checkout("feature", false).unwrap();
    let right = add(&repo, "doc.txt", b"top\nmiddle\nRIGHT\n", "change bottom");
    repo.checkout("main", false).unwrap();
    let merged = repo
        .merge("feature", "Developer", "merge branches")
        .unwrap();
    let merge_id = merged.revision.unwrap();
    assert!(merged.conflicts.is_empty());
    assert_eq!(
        fs::read(dir.path().join("doc.txt")).unwrap(),
        b"LEFT\nmiddle\nRIGHT\n"
    );
    assert_eq!(repo.history(None, 20).unwrap().len(), 4);
    let first = &repo.history(None, 1).unwrap()[0];
    assert_eq!(first.0, merge_id);
    assert_eq!(
        first
            .1
            .parents
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        [left, right]
    );

    repo.tag("v1", Some(&base), false).unwrap();
    let diff = repo.diff(Some("v1"), Some("HEAD"), false).unwrap();
    assert!(diff[0].text.as_ref().unwrap().contains("+RIGHT"));
    repo.restore("v1", &["doc.txt".into()]).unwrap();
    assert!(repo.status().unwrap().unstaged.contains_key("doc.txt"));
    repo.stage(&["doc.txt".into()]).unwrap();
    repo.unstage(&["doc.txt".into()]).unwrap();
    assert!(repo.status().unwrap().staged.is_empty());
    repo.restore("HEAD", &["doc.txt".into()]).unwrap();
    fs::write(dir.path().join(".lvcsignore"), "*.tmp\n!important.tmp\n").unwrap();
    fs::write(dir.path().join("discard.tmp"), "ignored").unwrap();
    fs::write(dir.path().join("important.tmp"), "keep").unwrap();
    let status = repo.status().unwrap();
    assert!(!status.unstaged.contains_key("discard.tmp"));
    assert!(status.unstaged.contains_key("important.tmp"));
    repo.gc(Duration::ZERO).unwrap();
    drop(repo);
    let repo = Repository::open(dir.path()).unwrap();
    repo.verify().unwrap();
    repo.checkout("v1", true).unwrap();
    assert_eq!(
        fs::read(dir.path().join("doc.txt")).unwrap(),
        b"top\nmiddle\nbottom\n"
    );
}

#[test]
fn binary_conflicts_keep_ours_and_require_explicit_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), Settings::default()).unwrap();
    add(&repo, "asset.bin", b"\0base", "base");
    repo.branch("other", None).unwrap();
    add(&repo, "asset.bin", b"\0ours", "ours");
    repo.checkout("other", false).unwrap();
    add(&repo, "asset.bin", b"\0other", "other");
    repo.checkout("main", false).unwrap();
    let merge = repo.merge("other", "Developer", "combine").unwrap();
    assert_eq!(
        merge.conflicts.into_iter().collect::<Vec<_>>(),
        ["asset.bin"]
    );
    assert_eq!(fs::read(dir.path().join("asset.bin")).unwrap(), b"\0ours");
    assert!(repo.commit("Developer", "cannot commit").is_err());
    repo.abort_merge().unwrap();
    repo.merge("other", "Developer", "combine").unwrap();
    fs::write(dir.path().join("asset.bin"), b"\0chosen").unwrap();
    repo.stage(&["asset.bin".into()]).unwrap();
    repo.commit("Developer", "resolved").unwrap();
    assert_eq!(repo.history(None, 1).unwrap()[0].1.parents.len(), 2);
}

#[test]
fn switching_preserves_untracked_and_modified_files() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), Settings::default()).unwrap();
    add(&repo, "doc.txt", b"base", "base");
    repo.branch("other", None).unwrap();
    repo.checkout("other", false).unwrap();
    add(&repo, "new.txt", b"branch file", "add file");
    repo.checkout("main", false).unwrap();
    fs::write(dir.path().join("new.txt"), "untracked").unwrap();
    assert!(repo.checkout("other", false).is_err());
    assert_eq!(fs::read(dir.path().join("new.txt")).unwrap(), b"untracked");
    assert_eq!(repo.status().unwrap().branch.as_deref(), Some("main"));
}

#[test]
fn checkout_between_a_file_and_a_directory() {
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), Settings::default()).unwrap();
    add(&repo, "data", b"file", "file");
    repo.branch("other", None).unwrap();
    repo.checkout("other", false).unwrap();
    fs::remove_file(dir.path().join("data")).unwrap();
    fs::create_dir_all(dir.path().join("data/nested")).unwrap();
    fs::write(dir.path().join("data/nested/file"), b"file in directory").unwrap();
    repo.stage(&["data".into()]).unwrap();
    repo.commit("Developer", "nested file").unwrap();
    repo.checkout("main", false).unwrap();
    assert_eq!(fs::read(dir.path().join("data")).unwrap(), b"file");
    repo.checkout("other", false).unwrap();
    assert_eq!(
        fs::read(dir.path().join("data/nested/file")).unwrap(),
        b"file in directory"
    );
}

#[test]
fn interrupted_checkout_replays_before_open_returns() {
    use serde_json::{Value, json};
    let dir = tempfile::tempdir().unwrap();
    let repo = Repository::init(dir.path(), Settings::default()).unwrap();
    add(&repo, "first", b"main file", "base");
    repo.branch("feature", None).unwrap();
    repo.checkout("feature", false).unwrap();
    fs::remove_file(repo.root().join("first")).unwrap();
    fs::write(repo.root().join("second"), "feature file").unwrap();
    repo.stage(&[".".into()]).unwrap();
    repo.commit("Developer", "feature").unwrap();
    let metadata = repo.root().join(".lvcs");
    let state = || -> Value {
        serde_json::from_slice(&fs::read(metadata.join("state.json")).unwrap()).unwrap()
    };
    let target = state();
    repo.checkout("main", false).unwrap();
    let initial = state();
    drop(repo);

    // The operation persisted its intent and removed one path before interruption.
    loki_vcs_core::atomic_json(
        &metadata.join("checkout.json"),
        &json!({
            "old": initial["index"], "new": target["index"], "state": target
        }),
    )
    .unwrap();
    fs::remove_file(dir.path().join("first")).unwrap();
    let recovered = Repository::open(dir.path()).unwrap();
    assert_eq!(
        recovered.status().unwrap().branch.as_deref(),
        Some("feature")
    );
    assert_eq!(
        fs::read(dir.path().join("second")).unwrap(),
        b"feature file"
    );
    assert!(!metadata.join("checkout.json").exists());
    recovered.verify().unwrap();
}
