//! Tests for embedding binarization logic.

use strata_agent::embed::binarize;

#[test]
fn binarize_produces_256_bit_embedding() {
    // Create a simple 256-dim float vector: first half negative, second half positive.
    let mut floats = vec![0.0f32; 256];
    for i in 0..128 {
        floats[i] = -1.0;
    }
    for i in 128..256 {
        floats[i] = 1.0;
    }

    let emb = binarize(&floats);
    let words = emb.into_inner();

    // First 128 bits should be 0 (below median), last 128 should be 1 (above median).
    // Word 0 covers bits 0-63, Word 1 covers 64-127 → both should be 0.
    // Word 2 covers bits 128-191, Word 3 covers 192-255 → both should be all 1s.
    assert_eq!(words[0], 0);
    assert_eq!(words[1], 0);
    assert_eq!(words[2], u64::MAX);
    assert_eq!(words[3], u64::MAX);
}

#[test]
fn binarize_handles_longer_vectors() {
    // Embedding APIs return 1536 dims but we only use first 256.
    let mut floats = vec![0.0f32; 1536];
    for i in 0..256 {
        floats[i] = if i % 2 == 0 { 1.0 } else { -1.0 };
    }

    let emb = binarize(&floats);
    let words = emb.into_inner();

    // Even bits set, odd bits unset in each word.
    // Pattern: ...01010101 = 0x5555555555555555
    for word in &words {
        assert_eq!(*word, 0x5555555555555555);
    }
}

#[test]
fn binarize_all_same_produces_zeros() {
    // When all values are equal (all at median), none is strictly above → all bits 0.
    let floats = vec![0.5f32; 256];
    let emb = binarize(&floats);
    let words = emb.into_inner();
    for word in &words {
        assert_eq!(*word, 0);
    }
}

#[test]
fn binarize_empty_input_returns_default() {
    let emb = binarize(&[]);
    assert_eq!(emb, strata_core::BinaryEmbedding::default());
}

#[test]
fn binarize_single_element() {
    let emb = binarize(&[1.0]);
    // Single element: median is sorted[0] = 1.0, nothing strictly above → all zeros.
    assert_eq!(emb, strata_core::BinaryEmbedding::default());
}
