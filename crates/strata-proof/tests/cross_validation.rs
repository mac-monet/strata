//! Cross-validation: guest MMR simulation vs real commonware `Mmr`.
//!
//! Appends entries to a real `journaled::Mmr` and to the guest's
//! `simulate_appends`, then asserts both produce identical roots.

use commonware_codec::Encode;
use commonware_cryptography::Hasher as _;
use commonware_runtime::{Metrics, Runner as _, deterministic};
use commonware_storage::mmr::{StandardHasher, journaled};
use std::num::{NonZeroU16, NonZeroU64, NonZeroUsize};
use strata_core::{
    BinaryEmbedding, ContentHash, CoreState, MemoryEntry, MemoryId, Nonce, SoulHash, VectorRoot,
};
use strata_proof::{Keccak256Hasher, Witness, compute_root, simulate_appends, transition};
use strata_vector_db::keccak::{Digest, Keccak256};

fn make_config(suffix: &str, context: &deterministic::Context) -> journaled::Config {
    let page_size = NonZeroU16::new(4096).unwrap();
    let page_cache_size = NonZeroUsize::new(8).unwrap();

    journaled::Config {
        journal_partition: format!("guest-test-journal-{suffix}"),
        metadata_partition: format!("guest-test-meta-{suffix}"),
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
        BinaryEmbedding::test_from_id(id),
        ContentHash::digest(text),
    )
}

type KeccakDigest = <Keccak256 as commonware_cryptography::Hasher>::Digest;

fn digest_to_bytes(d: &KeccakDigest) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(d.as_ref());
    bytes
}

/// Verify guest empty root matches real MMR empty root.
#[test]
fn empty_root_matches_standard_hasher() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("empty-root", &context);
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mmr: journaled::Mmr<_, KeccakDigest> =
            journaled::Mmr::init(context, &mut hasher, config)
                .await
                .unwrap();

        let real_root = digest_to_bytes(&mmr.root());
        let guest_root = compute_root::<Keccak256Hasher>(0, &[]);

        assert_eq!(
            real_root, guest_root,
            "guest empty root must match real MMR empty root"
        );

        mmr.destroy().await.unwrap();
    });
}

/// Verify guest simulation matches real MMR for a single leaf.
#[test]
fn single_leaf_matches() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("single", &context);
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mut mmr: journaled::Mmr<_, KeccakDigest> =
            journaled::Mmr::init(context, &mut hasher, config)
                .await
                .unwrap();

        let entry = make_entry(0, b"hello");
        let leaf_bytes = entry.encode().to_vec();

        // Real MMR append
        {
            let mut batch = mmr.new_batch();
            batch.add(&mut hasher, &leaf_bytes);
            let changeset = batch.merkleize(&mut hasher).finalize();
            mmr.apply(changeset).unwrap();
        }
        let real_root = digest_to_bytes(&mmr.root());

        // Guest simulation
        let mut peaks = Vec::new();
        let mut count = 0u64;
        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &[leaf_bytes]);
        let guest_root = compute_root::<Keccak256Hasher>(count, &peaks);

        assert_eq!(real_root, guest_root, "single-leaf roots must match");

        mmr.destroy().await.unwrap();
    });
}

/// Verify guest simulation matches real MMR for multiple leaves.
#[test]
fn multi_leaf_matches() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("multi", &context);
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mut mmr: journaled::Mmr<_, KeccakDigest> =
            journaled::Mmr::init(context, &mut hasher, config)
                .await
                .unwrap();

        let entries: Vec<MemoryEntry> = (0..7)
            .map(|i| make_entry(i, format!("entry-{i}").as_bytes()))
            .collect();
        let entries_bytes: Vec<Vec<u8>> = entries.iter().map(|e| e.encode().to_vec()).collect();

        // Real MMR: append all
        {
            let mut batch = mmr.new_batch();
            for eb in &entries_bytes {
                batch.add(&mut hasher, eb);
            }
            let changeset = batch.merkleize(&mut hasher).finalize();
            mmr.apply(changeset).unwrap();
        }
        let real_root = digest_to_bytes(&mmr.root());

        // Guest simulation from scratch
        let mut peaks = Vec::new();
        let mut count = 0u64;
        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &entries_bytes);
        let guest_root = compute_root::<Keccak256Hasher>(count, &peaks);

        assert_eq!(
            real_root, guest_root,
            "7-leaf roots must match (popcount(7)=3 peaks)"
        );

        mmr.destroy().await.unwrap();
    });
}

/// Verify incremental guest append matches real MMR.
#[test]
fn incremental_append_matches() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("incremental", &context);
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mut mmr: journaled::Mmr<_, KeccakDigest> =
            journaled::Mmr::init(context, &mut hasher, config)
                .await
                .unwrap();

        let batch1: Vec<MemoryEntry> = (0..3)
            .map(|i| make_entry(i, format!("b1-{i}").as_bytes()))
            .collect();
        let batch2: Vec<MemoryEntry> = (3..5)
            .map(|i| make_entry(i, format!("b2-{i}").as_bytes()))
            .collect();

        let batch1_bytes: Vec<Vec<u8>> = batch1.iter().map(|e| e.encode().to_vec()).collect();
        let batch2_bytes: Vec<Vec<u8>> = batch2.iter().map(|e| e.encode().to_vec()).collect();

        // Real MMR: append batch 1
        {
            let mut batch = mmr.new_batch();
            for eb in &batch1_bytes {
                batch.add(&mut hasher, eb);
            }
            let changeset = batch.merkleize(&mut hasher).finalize();
            mmr.apply(changeset).unwrap();
        }
        let mid_root_real = digest_to_bytes(&mmr.root());

        // Real MMR: append batch 2
        {
            let mut batch = mmr.new_batch();
            for eb in &batch2_bytes {
                batch.add(&mut hasher, eb);
            }
            let changeset = batch.merkleize(&mut hasher).finalize();
            mmr.apply(changeset).unwrap();
        }
        let final_root_real = digest_to_bytes(&mmr.root());

        // Guest: simulate batch 1
        let mut peaks = Vec::new();
        let mut count = 0u64;
        simulate_appends::<Keccak256Hasher>(&mut peaks, &mut count, &batch1_bytes);
        let mid_root_guest = compute_root::<Keccak256Hasher>(count, &peaks);
        assert_eq!(mid_root_real, mid_root_guest, "mid roots must match");

        // Guest: verify_append with batch 2
        let final_root_guest = strata_proof::verify_append::<Keccak256Hasher>(
            &peaks,
            count,
            &mid_root_guest,
            &batch2_bytes,
        )
        .unwrap();
        assert_eq!(
            final_root_real, final_root_guest,
            "final roots must match after incremental append"
        );

        mmr.destroy().await.unwrap();
    });
}

/// End-to-end: full transition() call with real MMR root comparison.
#[test]
fn full_transition_matches_real_mmr() {
    deterministic::Runner::default().start(|context| async move {
        let config = make_config("transition", &context);
        let mut hasher = StandardHasher::<Keccak256>::new();
        let mut mmr: journaled::Mmr<_, KeccakDigest> =
            journaled::Mmr::init(context, &mut hasher, config)
                .await
                .unwrap();

        // Genesis: empty MMR root
        let genesis_root = digest_to_bytes(&mmr.root());
        let guest_genesis = compute_root::<Keccak256Hasher>(0, &[]);
        assert_eq!(genesis_root, guest_genesis);

        let state = CoreState {
            soul_hash: SoulHash::digest(b"test-soul"),
            vector_index_root: VectorRoot::new(genesis_root),
            nonce: Nonce::new(0),
        };

        // Append entries to real MMR
        let entries: Vec<MemoryEntry> = (0..4)
            .map(|i| make_entry(i, format!("mem-{i}").as_bytes()))
            .collect();
        let entries_bytes: Vec<Vec<u8>> = entries.iter().map(|e| e.encode().to_vec()).collect();

        {
            let mut batch = mmr.new_batch();
            for eb in &entries_bytes {
                batch.add(&mut hasher, eb);
            }
            let changeset = batch.merkleize(&mut hasher).finalize();
            mmr.apply(changeset).unwrap();
        }
        let expected_new_root = digest_to_bytes(&mmr.root());

        // Run guest transition
        let witness = Witness {
            old_peaks: vec![], // empty MMR has 0 peaks
            old_leaf_count: 0,
            new_entries: entries,
        };

        let new_state =
            transition::<Keccak256Hasher>(state, Nonce::new(1), &witness).unwrap();

        assert_eq!(
            new_state.vector_index_root.as_bytes(),
            &expected_new_root,
            "guest transition root must match real MMR root"
        );
        assert_eq!(new_state.nonce, Nonce::new(1));
        assert_eq!(new_state.soul_hash, state.soul_hash);

        mmr.destroy().await.unwrap();
    });
}
