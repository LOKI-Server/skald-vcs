use skald_vcs_core::{Id, Kind, Revision, Tree, canonical, now, protocol::Update};
use skald_vcs_server::{Conflict, Hosted};
use std::sync::{Arc, Barrier};

fn commit(host: &Hosted, parents: Vec<Id>, message: &str) -> Id {
    let root = host
        .store()
        .put(Kind::Tree, &canonical(&Tree::new()).unwrap())
        .unwrap();
    host.store()
        .put(
            Kind::Revision,
            &canonical(&Revision {
                parents,
                root,
                author: "Developer".into(),
                timestamp: now(),
                message: message.into(),
            })
            .unwrap(),
        )
        .unwrap()
}

#[test]
fn stale_concurrent_push_loses_and_incomplete_upload_can_resume() {
    let dir = tempfile::tempdir().unwrap();
    let host = Hosted::create(&dir.path().join("remote"), "main").unwrap();
    let tree = canonical(&Tree::new()).unwrap();
    let root = Id::of(Kind::Tree, &tree);
    let raw = canonical(&Revision {
        root,
        parents: vec![],
        author: "Developer".into(),
        timestamp: now(),
        message: "initial".into(),
    })
    .unwrap();
    let first = Id::of(Kind::Revision, &raw);
    host.receive(first, &raw, |_| Ok(())).unwrap();
    assert!(
        host.update(
            false,
            "main",
            Update {
                expected: None,
                target: Some(first)
            }
        )
        .is_err()
    );
    assert_eq!(host.missing(vec![first, root]).unwrap(), [root]);
    assert!(host.refs().unwrap().branches.is_empty());
    host.receive(root, &tree, |_| Ok(())).unwrap();
    host.update(
        false,
        "main",
        Update {
            expected: None,
            target: Some(first),
        },
    )
    .unwrap();

    let next = [
        commit(&host, vec![first], "one"),
        commit(&host, vec![first], "two"),
    ];
    let gate = Arc::new(Barrier::new(2));
    let results: Vec<_> = next
        .into_iter()
        .map(|target| {
            let (host, gate) = (host.clone(), gate.clone());
            std::thread::spawn(move || {
                gate.wait();
                host.update(
                    false,
                    "main",
                    Update {
                        expected: Some(first),
                        target: Some(target),
                    },
                )
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|t| t.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|r| r.as_ref().is_err_and(|e| e.is::<Conflict>()))
            .count(),
        1
    );
    host.verify().unwrap();
    assert!(
        host.lock("Assets/image", "alice", Some(60))
            .unwrap()
            .is_some()
    );
    assert!(
        host.lock("assets/image", "bob", Some(60))
            .unwrap_err()
            .is::<Conflict>()
    );
    assert!(host.lock("Assets/image", "alice", None).unwrap().is_none());
    assert!(host.locks().unwrap().is_empty());
}
