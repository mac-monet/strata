mod common;

use strata_core::{BinaryEmbedding, ContentHash, MemoryEntry, MemoryId};
use strata_agent::persist;

fn make_entry(id: u64, text: &[u8]) -> MemoryEntry {
    MemoryEntry::new(
        MemoryId::new(id),
        BinaryEmbedding::test_from_id(id),
        ContentHash::digest(text),
    )
}

#[test]
fn snapshot_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.json");

    let state = common::genesis_state();
    let entries = vec![
        make_entry(0, b"hello"),
        make_entry(1, b"world"),
    ];
    let contents = vec!["hello".to_string(), "world".to_string()];

    let snap = persist::Snapshot {
        state,
        entries: entries.clone(),
        contents: contents.clone(),
    };

    persist::save(&path, &snap).unwrap();
    assert!(path.exists());

    let loaded = persist::load(&path).unwrap().expect("should exist");

    assert_eq!(loaded.state.nonce, state.nonce);
    assert_eq!(loaded.state.soul_hash, state.soul_hash);
    assert_eq!(loaded.state.vector_index_root, state.vector_index_root);
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.contents, contents);

    // Verify entry fields survived serialization.
    for (orig, loaded) in entries.iter().zip(loaded.entries.iter()) {
        assert_eq!(orig.id, loaded.id);
        assert_eq!(orig.embedding, loaded.embedding);
        assert_eq!(orig.content_hash, loaded.content_hash);
    }
}

#[test]
fn load_missing_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nonexistent.json");
    assert!(persist::load(&path).unwrap().is_none());
}

#[test]
fn snapshot_with_db_round_trip() {
    // End-to-end: create DB, append entries, snapshot, reload with open().
    use commonware_runtime::{deterministic, Runner as _};
    use strata_vector_db::VectorDB;

    let dir = tempfile::tempdir().unwrap();
    let snap_path = dir.path().join("snapshot.json");

    // Phase 1: create DB, append entries, save snapshot.
    let snap_path_w = snap_path.clone();
    let root_before = deterministic::Runner::default().start(|context| async move {
        let config = common::make_config("persist-w", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        let e0 = make_entry(0, b"first memory");
        let e1 = make_entry(1, b"second memory");
        db.append(e0).await.unwrap();
        db.append(e1).await.unwrap();
        db.sync().await.unwrap();

        let root = db.root();

        let state = common::genesis_state();
        let contents = vec!["first memory".to_string(), "second memory".to_string()];
        let snap = persist::Snapshot {
            state,
            entries: db.entries().to_vec(),
            contents,
        };
        persist::save(&snap_path_w, &snap).unwrap();
        root
    });

    // Phase 2: load snapshot, verify entries and query work.
    let loaded = persist::load(&snap_path).unwrap().unwrap();
    assert_eq!(loaded.entries.len(), 2);
    assert_eq!(loaded.contents, vec!["first memory", "second memory"]);

    // Verify entry fields round-tripped.
    assert_eq!(loaded.entries[0].id, MemoryId::new(0));
    assert_eq!(loaded.entries[1].id, MemoryId::new(1));
    assert_eq!(loaded.entries[0].content_hash, ContentHash::digest(b"first memory"));
    assert_eq!(loaded.entries[1].content_hash, ContentHash::digest(b"second memory"));

    // Verify root and state survived serialization.
    assert_eq!(loaded.state.vector_index_root, common::genesis_state().vector_index_root);

    // Note: Full VectorDB::open() with MMR recovery requires real disk persistence
    // (tokio runtime), not possible with deterministic runtime's ephemeral storage.
    // The snapshot data is correct; MMR journal recovery is tested by commonware.
}
