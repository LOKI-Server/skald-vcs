use skald_vcs_core::{
    Id, Kind, MAX_OBJECT, Manifest, Object, Settings, Store, canonical, valid_path,
};
use std::{collections::HashSet, fs, io::Cursor};

fn fixture(size: usize) -> Vec<u8> {
    let mut state = 0x68310f56_u32;
    (0..size)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect()
}

#[test]
fn shifted_binary_reuses_chunks_and_reconstructs_exactly() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::create(folder.path(), Settings::default()).unwrap();
    let original = fixture(12 * 1024 * 1024);
    let initial = store.write_file(Cursor::new(&original)).unwrap();
    let previous: HashSet<_> = store
        .manifest(initial)
        .unwrap()
        .chunks
        .into_iter()
        .map(|c| c.id)
        .collect();
    let mut changed = original.clone();
    changed.splice(300_000..300_000, [1, 2, 3, 4, 5]);
    let revision = store.write_file(Cursor::new(&changed)).unwrap();
    let manifest = store.manifest(revision).unwrap();
    let reuse: u64 = manifest
        .chunks
        .iter()
        .filter(|c| previous.contains(&c.id))
        .map(|c| u64::from(c.size))
        .sum();
    assert!(reuse > changed.len() as u64 * 8 / 10);
    let mut out = Vec::new();
    store.copy_file(revision, &mut out).unwrap();
    assert_eq!(out, changed);
    let count = store.object_ids().unwrap().len();
    assert_eq!(store.write_file(Cursor::new(&original)).unwrap(), initial);
    assert_eq!(store.object_ids().unwrap().len(), count);
    store.verify([initial, revision]).unwrap();
}

#[test]
fn rejects_corruption_and_malformed_metadata() {
    let folder = tempfile::tempdir().unwrap();
    let store = Store::create(folder.path(), Settings::default()).unwrap();
    let id = store.put(Kind::Chunk, b"good").unwrap();
    assert!(store.receive(id, b"bad").is_err());
    let name = id.to_string();
    fs::write(folder.path().join(&name[..3]).join(&name[3..]), b"\0bad").unwrap();
    assert!(store.verify([id]).is_err());
    let bad = Manifest {
        size: 3,
        chunks: vec![],
    };
    let bytes = canonical(&bad).unwrap();
    assert!(Object::parse(Id::of(Kind::File, &bytes), &bytes).is_err());
    let bytes = vec![1; MAX_OBJECT + 1];
    assert!(Object::parse(Id::of(Kind::Chunk, &bytes), &bytes).is_err());
    for path in [
        "a/../b",
        "a\\b",
        "CON.txt",
        "a/.SKALD/x",
        "a//b",
        "d /x",
        "/root",
        "a:stream",
    ] {
        assert!(valid_path(path).is_err(), "{path}");
    }
}

#[test]
fn parser_round_trips_generated_manifests_and_rejects_mutations() {
    use skald_vcs_core::Chunk;
    for length in 1..=128u32 {
        let chunks: Vec<_> = (1..=length)
            .map(|size| Chunk {
                id: Id::of(Kind::Chunk, &size.to_le_bytes()),
                size,
            })
            .collect();
        let manifest = Manifest {
            size: chunks.iter().map(|c| u64::from(c.size)).sum(),
            chunks,
        };
        let mut bytes = canonical(&manifest).unwrap();
        let id = Id::of(Kind::File, &bytes);
        let Ok(Object::File(roundtrip)) = Object::parse(id, &bytes) else {
            panic!("valid manifest rejected")
        };
        assert_eq!(manifest, roundtrip);
        for i in [0, bytes.len() / 2, bytes.len() - 1] {
            bytes[i] ^= 1;
            assert!(Object::parse(id, &bytes).is_err());
            bytes[i] ^= 1;
        }
        bytes.extend_from_slice(b" ");
        assert!(Object::parse(Id::of(Kind::File, &bytes), &bytes).is_err());
    }
}
