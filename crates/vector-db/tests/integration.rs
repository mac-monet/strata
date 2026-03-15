use commonware_codec::Encode;
use commonware_cryptography::Hasher as _;
use commonware_runtime::{Metrics, Runner as _, deterministic};
use commonware_storage::mmr::{Location, StandardHasher, journaled};
use std::num::{NonZeroU64, NonZeroU16, NonZeroUsize};
use strata_core::{BinaryEmbedding, ContentHash, MemoryEntry, MemoryId, VectorRoot};
use strata_vector_db::VectorDB;
use strata_vector_db::keccak::{Digest, Keccak256};

fn make_config(suffix: &str, context: &deterministic::Context) -> journaled::Config {
    let page_size = NonZeroU16::new(4096).unwrap();
    let page_cache_size = NonZeroUsize::new(8).unwrap();

    journaled::Config {
        journal_partition: format!("vdb-journal-{suffix}"),
        metadata_partition: format!("vdb-meta-{suffix}"),
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

fn make_entry(id: u64, text: &[u8]) -> MemoryEntry {
    MemoryEntry::new(
        MemoryId::new(id),
        BinaryEmbedding::new([id, id + 1, id + 2, id + 3]),
        ContentHash::digest(text),
    )
}

#[test]
fn new_returns_empty() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("empty", &context);
        let db = VectorDB::new(context, config).await.unwrap();

        assert_eq!(db.len(), 0);
        assert!(db.is_empty());
        // Root should be the empty MMR root (non-zero due to position-aware hashing).
        assert_ne!(db.root(), VectorRoot::default());
    });
}

#[test]
fn append_changes_root_and_increments_len() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("append", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        let root_before = db.root();
        let entry = make_entry(0, b"hello world");
        let root_after = db.append(entry).await.unwrap();

        assert_ne!(root_before, root_after);
        assert_eq!(db.len(), 1);
        assert!(!db.is_empty());
    });
}

#[test]
fn batch_append_matches_sequential() {
    deterministic::Runner::default().start(|context| async move {
        let entries = vec![
            make_entry(0, b"alpha"),
            make_entry(1, b"beta"),
            make_entry(2, b"gamma"),
        ];

        // Sequential appends
        let ctx_seq = context.with_label("seq");
        let config_seq = make_config("seq", &ctx_seq);
        let mut db_seq = VectorDB::new(ctx_seq, config_seq).await.unwrap();
        for entry in &entries {
            db_seq.append(*entry).await.unwrap();
        }
        let root_seq = db_seq.root();

        // Batch append
        let ctx_batch = context.with_label("batch");
        let config_batch = make_config("batch", &ctx_batch);
        let mut db_batch = VectorDB::new(ctx_batch, config_batch).await.unwrap();
        let root_batch = db_batch.batch_append(entries).await.unwrap();

        assert_eq!(root_seq, root_batch);

        db_seq.destroy().await.unwrap();
        db_batch.destroy().await.unwrap();
    });
}

#[test]
fn query_returns_correct_top_k() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("query", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        // Entry 0: embedding is [0, 1, 2, 3]
        db.append(make_entry(0, b"a")).await.unwrap();
        // Entry 1: embedding is [1, 2, 3, 4]
        db.append(make_entry(1, b"b")).await.unwrap();
        // Entry 2: embedding is [2, 3, 4, 5]
        db.append(make_entry(2, b"c")).await.unwrap();

        // Query with embedding [0, 1, 2, 3] — should match entry 0 exactly
        let query = BinaryEmbedding::new([0, 1, 2, 3]);
        let results = db.query(&query, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entry.id, MemoryId::new(0));
        assert_eq!(results[0].distance, 0);

        db.destroy().await.unwrap();
    });
}

#[test]
fn query_k_larger_than_len() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("query-big-k", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        db.append(make_entry(0, b"only one")).await.unwrap();

        let query = BinaryEmbedding::new([0, 0, 0, 0]);
        let results = db.query(&query, 100);
        assert_eq!(results.len(), 1);

        db.destroy().await.unwrap();
    });
}

#[test]
fn get_returns_correct_entry() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("get", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        let entry0 = make_entry(0, b"first");
        let entry1 = make_entry(1, b"second");
        db.append(entry0).await.unwrap();
        db.append(entry1).await.unwrap();

        assert_eq!(db.get(MemoryId::new(0)), Some(&entry0));
        assert_eq!(db.get(MemoryId::new(1)), Some(&entry1));
        assert_eq!(db.get(MemoryId::new(99)), None);

        db.destroy().await.unwrap();
    });
}

#[test]
fn witness_generates_valid_proof() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("witness", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();

        // Append 4 entries
        for i in 0..4u64 {
            db.append(make_entry(i, format!("entry-{i}").as_bytes()))
                .await
                .unwrap();
        }

        // Witness from leaf 2 onwards (entries 2 and 3 are new)
        let witness = db.witness(2).await.unwrap();
        assert_eq!(witness.new_entries.len(), 2);
        assert_eq!(witness.new_root, db.root());
        assert_eq!(witness.start_location, Location::new(2));

        // Verify the proof against the current root
        let mut hasher = StandardHasher::<Keccak256>::new();
        let elements: Vec<Vec<u8>> = witness
            .new_entries
            .iter()
            .map(|e| e.encode().to_vec())
            .collect();
        let element_refs: Vec<&[u8]> = elements.iter().map(|e| e.as_slice()).collect();
        let root_digest = Digest(*witness.new_root.as_bytes());
        let valid = witness.proof.verify_range_inclusion(
            &mut hasher,
            &element_refs,
            witness.start_location,
            &root_digest,
        );
        assert!(valid, "witness proof should verify");

        db.destroy().await.unwrap();
    });
}

#[test]
fn witness_no_new_entries_returns_error() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("witness-err", &context);
        let mut db = VectorDB::new(context, config).await.unwrap();
        db.append(make_entry(0, b"only")).await.unwrap();

        let result = db.witness(1).await;
        assert!(result.is_err());

        db.destroy().await.unwrap();
    });
}

#[test]
fn sync_and_reopen_preserves_root() {
    deterministic::Runner::default().start(|context| async move {
        let entry0 = make_entry(0, b"alpha");
        let entry1 = make_entry(1, b"beta");
        let root_before;
        let len_before;

        // First session: create, append, sync
        {
            let ctx1 = context.with_label("session1");
            let config = make_config("persist", &ctx1);
            let mut db = VectorDB::new(ctx1, config).await.unwrap();
            db.append(entry0).await.unwrap();
            db.append(entry1).await.unwrap();
            root_before = db.root();
            len_before = db.len();
            db.sync().await.unwrap();
        }

        // Second session: reopen with same partition names
        {
            let ctx2 = context.with_label("session2");
            let config = make_config("persist", &ctx2);
            let db2 = VectorDB::open(ctx2, config, vec![entry0, entry1])
                .await
                .unwrap();
            assert_eq!(db2.root(), root_before);
            assert_eq!(db2.len(), len_before);
        }
    });
}

#[test]
fn open_rejects_index_mismatch() {
    deterministic::Runner::default().start(|context| async move {
        let entry0 = make_entry(0, b"alpha");
        let entry1 = make_entry(1, b"beta");

        // First session: create with 2 entries, sync
        {
            let ctx1 = context.with_label("session1");
            let config = make_config("mismatch", &ctx1);
            let mut db = VectorDB::new(ctx1, config).await.unwrap();
            db.append(entry0).await.unwrap();
            db.append(entry1).await.unwrap();
            db.sync().await.unwrap();
        }

        // Second session: reopen with wrong number of entries
        {
            let ctx2 = context.with_label("session2");
            let config = make_config("mismatch", &ctx2);
            let result = VectorDB::open(ctx2, config, vec![entry0]).await;
            assert!(result.is_err(), "should reject mismatched entry count");
        }
    });
}
