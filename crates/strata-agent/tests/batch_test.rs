//! Tests for the batch module (WAL, public values, reconstruction encoding).

use std::path::PathBuf;
use std::sync::Arc;

use commonware_codec::Encode;
use strata_core::*;
use strata_proof::Witness;

use strata_agent::batch::{self, PendingBatch};
use strata_agent::pipeline::{self, TransitionOutput};

/// Build a minimal test transition with the given nonces.
fn make_transition(old_nonce: u64, new_nonce: u64) -> TransitionOutput {
    let soul_hash = SoulHash::digest(b"test-soul");
    let old_root = [old_nonce as u8; 32];
    let new_root = [new_nonce as u8; 32];

    let entry = MemoryEntry {
        id: MemoryId::new(old_nonce),
        content_hash: ContentHash::new([42u8; 32]),
        embedding: BinaryEmbedding::new([0u64; 16]),
    };

    TransitionOutput {
        record: TransitionRecord::new(
            Input::new(
                Nonce::new(new_nonce),
                InputPayload::MemoryUpdate,
                InputSignature::default(),
            ),
            vec![entry],
            vec![MemoryContent::new(
                MemoryId::new(old_nonce),
                format!("memory-{new_nonce}").into_bytes(),
            )],
        ),
        witness: Witness {
            old_peaks: vec![[99u8; 32]],
            old_leaf_count: old_nonce,
            new_entries: vec![entry],
        },
        old_state: CoreState {
            soul_hash,
            vector_index_root: VectorRoot::new(old_root),
            nonce: Nonce::new(old_nonce),
        },
        new_state: CoreState {
            soul_hash,
            vector_index_root: VectorRoot::new(new_root),
            nonce: Nonce::new(new_nonce),
        },
    }
}

// ---------------------------------------------------------------------------
// WAL tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wal_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    let t1 = make_transition(0, 1);
    let t2 = make_transition(1, 2);

    // Write two transitions to WAL.
    batch::write_wal(&wal_path, &[t1, t2]).await.unwrap();

    // Reload into a fresh buffer.
    let pending = Arc::new(PendingBatch::default());
    batch::reload_wal(&wal_path, &pending).await.unwrap();

    let loaded = pending.lock().await;
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].old_state.nonce.get(), 0);
    assert_eq!(loaded[0].new_state.nonce.get(), 1);
    assert_eq!(loaded[1].old_state.nonce.get(), 1);
    assert_eq!(loaded[1].new_state.nonce.get(), 2);

    // Verify record content survived serialization.
    assert_eq!(loaded[0].record.input.nonce.get(), 1);
    assert_eq!(loaded[1].record.input.nonce.get(), 2);
    assert_eq!(loaded[0].witness.old_leaf_count, 0);
    assert_eq!(loaded[1].witness.old_leaf_count, 1);
}

#[tokio::test]
async fn wal_overwrite_prevents_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    let batch_a = vec![make_transition(0, 1)];
    let batch_b = vec![make_transition(5, 6), make_transition(6, 7)];

    // Write first batch.
    batch::write_wal(&wal_path, &batch_a).await.unwrap();

    // Overwrite with second batch (simulating retry with new data).
    batch::write_wal(&wal_path, &batch_b).await.unwrap();

    // Reload — should contain only batch_b, not batch_a + batch_b.
    let pending = Arc::new(PendingBatch::default());
    batch::reload_wal(&wal_path, &pending).await.unwrap();

    let loaded = pending.lock().await;
    assert_eq!(loaded.len(), 2, "expected 2 transitions, got {}", loaded.len());
    assert_eq!(loaded[0].old_state.nonce.get(), 5);
    assert_eq!(loaded[1].old_state.nonce.get(), 6);
}

#[tokio::test]
async fn wal_empty_file_reloads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("test.wal");

    // Write then truncate (simulating successful post).
    let batch = vec![make_transition(0, 1)];
    batch::write_wal(&wal_path, &batch).await.unwrap();
    tokio::fs::write(&wal_path, b"").await.unwrap();

    let pending = Arc::new(PendingBatch::default());
    batch::reload_wal(&wal_path, &pending).await.unwrap();
    assert!(pending.lock().await.is_empty());
}

#[tokio::test]
async fn wal_missing_file_reloads_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let wal_path = dir.path().join("nonexistent.wal");

    let pending = Arc::new(PendingBatch::default());
    batch::reload_wal(&wal_path, &pending).await.unwrap();
    assert!(pending.lock().await.is_empty());
}

// ---------------------------------------------------------------------------
// Public values layout
// ---------------------------------------------------------------------------

#[test]
fn batch_public_values_layout() {
    let old_root = [0xAAu8; 32];
    let new_root = [0xBBu8; 32];
    let start_nonce: u64 = 5;
    let end_nonce: u64 = 10;
    let soul_hash = [0xCCu8; 32];

    let pv = pipeline::batch_public_values(&old_root, &new_root, start_nonce, end_nonce, &soul_hash);

    assert_eq!(pv.len(), 112);
    assert_eq!(&pv[0..32], &old_root);
    assert_eq!(&pv[32..64], &new_root);
    assert_eq!(&pv[64..72], &start_nonce.to_be_bytes());
    assert_eq!(&pv[72..80], &end_nonce.to_be_bytes());
    assert_eq!(&pv[80..112], &soul_hash);
}

#[test]
fn batch_public_values_single_transition() {
    // When start == end, it's equivalent to a single transition.
    let pv = pipeline::batch_public_values(&[1u8; 32], &[2u8; 32], 1, 1, &[3u8; 32]);
    let start = u64::from_be_bytes(pv[64..72].try_into().unwrap());
    let end = u64::from_be_bytes(pv[72..80].try_into().unwrap());
    assert_eq!(start, end);
    assert_eq!(start, 1);
}

// ---------------------------------------------------------------------------
// Reconstruction: batch encoding round-trip
// ---------------------------------------------------------------------------

#[test]
fn reconstruction_batch_decode_round_trip() {
    let t1 = make_transition(0, 1);
    let t2 = make_transition(1, 2);

    // Encode as length-prefixed batch (matching poster::post_batch format).
    let mut encoded = Vec::new();
    for t in [&t1, &t2] {
        let record_bytes = t.record.encode();
        encoded.extend_from_slice(&(record_bytes.len() as u32).to_be_bytes());
        encoded.extend_from_slice(&record_bytes);
    }

    // Decode via the reconstruction function.
    let cfg = TransitionRecordCfg::default();
    let records = strata_agent::reconstruct::decode_memory_content(&encoded, &cfg).unwrap();

    assert_eq!(records.len(), 2);
    assert_eq!(records[0].input.nonce.get(), 1);
    assert_eq!(records[1].input.nonce.get(), 2);
    assert_eq!(records[0].contents.len(), 1);
    assert_eq!(records[1].contents.len(), 1);
}

#[test]
fn reconstruction_empty_bytes_is_error() {
    let cfg = TransitionRecordCfg::default();
    let result = strata_agent::reconstruct::decode_memory_content(&[], &cfg);
    assert!(result.is_err());
}
