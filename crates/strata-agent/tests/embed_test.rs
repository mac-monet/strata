//! Tests for embedding binarization logic.

use strata_agent::embed::binarize;
use strata_core::EMBEDDING_WORDS;

#[test]
fn binarize_half_negative_half_positive() {
    // 1024-dim vector: first half negative, second half positive.
    let mut floats = vec![0.0f32; 1024];
    for i in 0..512 {
        floats[i] = -1.0;
    }
    for i in 512..1024 {
        floats[i] = 1.0;
    }

    let emb = binarize(&floats);
    let words = emb.into_inner();

    // First 512 bits (words 0-7) should be 0, last 512 bits (words 8-15) should be all 1s.
    for word in &words[..8] {
        assert_eq!(*word, 0);
    }
    for word in &words[8..] {
        assert_eq!(*word, u64::MAX);
    }
}

#[test]
fn binarize_handles_longer_vectors() {
    // Embedding APIs may return 1536 dims but we only use first 1024.
    let mut floats = vec![0.0f32; 1536];
    for i in 0..1024 {
        floats[i] = if i % 2 == 0 { 1.0 } else { -1.0 };
    }

    let emb = binarize(&floats);
    let words = emb.into_inner();

    // Even bits set, odd bits unset in each word → 0x5555555555555555.
    for word in &words {
        assert_eq!(*word, 0x5555555555555555);
    }
}

#[test]
fn binarize_shorter_model_zero_pads() {
    // A 768-dim model — only first 768 bits are populated, rest stay zero.
    let mut floats = vec![0.0f32; 768];
    for i in 0..768 {
        // Alternating so half are above median.
        floats[i] = if i % 2 == 0 { 1.0 } else { -1.0 };
    }

    let emb = binarize(&floats);
    let words = emb.into_inner();

    // Words 0-11 cover bits 0-767 (768 = 12 * 64), should have the alternating pattern.
    for word in &words[..12] {
        assert_eq!(*word, 0x5555555555555555);
    }
    // Words 12-15 cover bits 768-1023, should be zero (no data).
    for word in &words[12..] {
        assert_eq!(*word, 0);
    }
}

#[test]
fn binarize_all_same_produces_zeros() {
    let floats = vec![0.5f32; 1024];
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
    assert_eq!(emb, strata_core::BinaryEmbedding::default());
}

#[test]
fn binarize_is_deterministic() {
    let floats: Vec<f32> = (0..1024).map(|i| (i as f32).sin()).collect();
    let a = binarize(&floats);
    let b = binarize(&floats);
    assert_eq!(a, b);

    // Should have roughly half the bits set.
    let set_bits: u32 = a.as_words().iter().map(|w| w.count_ones()).sum();
    // With 1024 dims and median threshold, expect ~512 set bits.
    assert!(set_bits > 400 && set_bits < 624, "set_bits={set_bits}, expected ~512");
}
