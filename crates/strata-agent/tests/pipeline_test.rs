use commonware_runtime::{deterministic, Runner as _};
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Keccak256Hasher, compute_root};
use strata_vector_db::{Config as JournaledConfig, VectorDB};

use strata_agent::pipeline::{self, Snapshot};

fn make_config(suffix: &str, context: &deterministic::Context) -> JournaledConfig {
    let page_size = NonZeroU16::new(4096).unwrap();
    let page_cache_size = NonZeroUsize::new(8).unwrap();

    JournaledConfig {
        journal_partition: format!("pipeline-journal-{suffix}"),
        metadata_partition: format!("pipeline-meta-{suffix}"),
        items_per_blob: NonZeroU64::new(1000).unwrap(),
        write_buffer: NonZeroUsize::new(1024).unwrap(),
        thread_pool: None,
        page_cache: commonware_runtime::buffer::paged::CacheRef::from_pooler(
            context,
            page_size,
            page_cache_size,
        ),
    }
}

fn genesis_state() -> CoreState {
    let root = compute_root::<Keccak256Hasher>(0, &[]);
    CoreState {
        soul_hash: SoulHash::digest(b"test-soul"),
        vector_index_root: VectorRoot::new(root),
        nonce: Nonce::new(0),
    }
}

fn make_entry(id: u64, text: &[u8]) -> MemoryEntry {
    MemoryEntry::new(
        MemoryId::new(id),
        BinaryEmbedding::test_from_id(id),
        ContentHash::digest(text),
    )
}

#[test]
fn single_memory_transition_from_genesis() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("single", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let state = genesis_state();

        let snap = pipeline::snapshot(state, &db);

        // Remember one entry
        let entry = make_entry(0, b"hello world");
        db.append(entry).await.unwrap();
        let contents = vec!["hello world".to_string()];

        let output = pipeline::finalize(&snap, &db, &contents).unwrap();

        assert_eq!(output.public_values.len(), 104);
        assert_eq!(output.new_state.nonce, Nonce::new(1));
        assert_eq!(output.new_state.vector_index_root, db.root());
        assert_eq!(output.record.new_entries.len(), 1);

        db.destroy().await.unwrap();
    });
}

#[test]
fn multiple_memories_in_one_transition() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("multi", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let state = genesis_state();

        let snap = pipeline::snapshot(state, &db);

        // Remember three entries
        let texts = ["alpha", "beta", "gamma"];
        for (i, text) in texts.iter().enumerate() {
            let entry = make_entry(i as u64, text.as_bytes());
            db.append(entry).await.unwrap();
        }
        let contents: Vec<String> = texts.iter().map(|t| t.to_string()).collect();

        let output = pipeline::finalize(&snap, &db, &contents).unwrap();

        assert_eq!(output.record.new_entries.len(), 3);
        assert_eq!(output.record.contents.len(), 3);
        assert_eq!(output.new_state.nonce, Nonce::new(1));
        assert_eq!(output.new_state.vector_index_root, db.root());

        db.destroy().await.unwrap();
    });
}

#[test]
fn no_new_memories_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("no-new", &context);
        let db = VectorDB::new(context, config).await.unwrap();
        let state = genesis_state();

        let snap = pipeline::snapshot(state, &db);
        let contents: Vec<String> = vec![];

        let result = pipeline::finalize(&snap, &db, &contents);
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error, got Ok"),
        };
        assert!(err.contains("no new memories"), "got: {err}");

        db.destroy().await.unwrap();
    });
}

#[test]
fn public_values_byte_layout() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("layout", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let state = genesis_state();

        let snap = pipeline::snapshot(state, &db);
        let old_root = *state.vector_index_root.as_bytes();

        let entry = make_entry(0, b"test");
        db.append(entry).await.unwrap();
        let contents = vec!["test".to_string()];

        let output = pipeline::finalize(&snap, &db, &contents).unwrap();
        let pv = output.public_values;

        // [0..32] = old_root
        assert_eq!(&pv[0..32], &old_root);
        // [32..64] = new_root
        assert_eq!(&pv[32..64], db.root().as_bytes());
        // [64..72] = nonce as u64 big-endian
        assert_eq!(&pv[64..72], &1u64.to_be_bytes());
        // [72..104] = soul_hash
        assert_eq!(&pv[72..104], state.soul_hash.as_bytes());

        db.destroy().await.unwrap();
    });
}

#[test]
fn nonce_increments_from_nonzero() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("nonce-incr", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        // Pre-populate DB with 2 entries (simulating prior transitions)
        let entry0 = make_entry(0, b"prior-0");
        let entry1 = make_entry(1, b"prior-1");
        db.append(entry0).await.unwrap();
        db.append(entry1).await.unwrap();

        // State with nonce=5 and current DB root
        let state = CoreState {
            soul_hash: SoulHash::digest(b"test-soul"),
            vector_index_root: db.root(),
            nonce: Nonce::new(5),
        };

        let snap = pipeline::snapshot(state, &db);
        let mut contents = vec!["prior-0".to_string(), "prior-1".to_string()];

        // Add a new entry
        let entry2 = make_entry(2, b"new-entry");
        db.append(entry2).await.unwrap();
        contents.push("new-entry".to_string());

        let output = pipeline::finalize(&snap, &db, &contents).unwrap();

        assert_eq!(output.new_state.nonce, Nonce::new(6));
        assert_eq!(output.record.input.nonce, Nonce::new(6));
        // Verify nonce in public values
        assert_eq!(&output.public_values[64..72], &6u64.to_be_bytes());

        db.destroy().await.unwrap();
    });
}

#[test]
fn short_contents_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("short-contents", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let state = genesis_state();

        let snap = pipeline::snapshot(state, &db);

        let entry = make_entry(0, b"hello");
        db.append(entry).await.unwrap();

        // Pass empty contents — fewer than db.len()
        let result = pipeline::finalize(&snap, &db, &[]);
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error, got Ok"),
        };
        assert!(err.contains("contents length"), "got: {err}");

        db.destroy().await.unwrap();
    });
}

#[test]
fn chained_transitions() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("chained", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        let mut state = genesis_state();
        let mut contents: Vec<String> = Vec::new();

        // First transition: add one entry
        let snap1 = pipeline::snapshot(state, &db);
        let entry0 = make_entry(0, b"first");
        db.append(entry0).await.unwrap();
        contents.push("first".to_string());

        let out1 = pipeline::finalize(&snap1, &db, &contents).unwrap();
        assert_eq!(out1.new_state.nonce, Nonce::new(1));
        state = out1.new_state;

        // Second transition: add two more entries
        let snap2 = pipeline::snapshot(state, &db);
        let entry1 = make_entry(1, b"second");
        let entry2 = make_entry(2, b"third");
        db.append(entry1).await.unwrap();
        db.append(entry2).await.unwrap();
        contents.push("second".to_string());
        contents.push("third".to_string());

        let out2 = pipeline::finalize(&snap2, &db, &contents).unwrap();
        assert_eq!(out2.new_state.nonce, Nonce::new(2));
        assert_eq!(out2.record.new_entries.len(), 2);
        assert_eq!(out2.new_state.vector_index_root, db.root());

        // Verify continuity: out1's new_root == out2's old_root in public values
        assert_eq!(&out2.public_values[0..32], out1.new_state.vector_index_root.as_bytes());

        db.destroy().await.unwrap();
    });
}
