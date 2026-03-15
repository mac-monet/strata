//! Tests for the public embedding API.

use strata_agent::embed::{Threshold, binarize};
use strata_core::EMBEDDING_WORDS;

#[test]
fn binarize_median_alternating() {
    let mut floats = vec![0.0f32; 1024];
    for (i, v) in floats.iter_mut().enumerate() {
        *v = if i % 2 == 0 { 1.0 } else { -1.0 };
    }

    let emb = binarize(&floats, Threshold::Median);
    let words = emb.into_inner();
    for word in &words {
        assert_eq!(*word, 0x5555555555555555);
    }
}

#[test]
fn binarize_zero_vs_median_differ_on_biased_data() {
    // All values between 0.5 and 1.5 (all positive, not centered at zero).
    let floats: Vec<f32> = (0..1024).map(|i| 0.5 + (i as f32 / 1024.0)).collect();

    let zero = binarize(&floats, Threshold::Zero);
    let median = binarize(&floats, Threshold::Median);

    // Zero threshold: all values > 0 → all bits set.
    let zero_bits: u32 = zero.as_words().iter().map(|w| w.count_ones()).sum();
    assert_eq!(zero_bits, 1024);

    // Median threshold: ~half the bits set.
    let median_bits: u32 = median.as_words().iter().map(|w| w.count_ones()).sum();
    assert!(
        median_bits > 400 && median_bits < 624,
        "median_bits={median_bits}, expected ~512"
    );
}

#[test]
fn binarize_handles_longer_vectors() {
    // 1536-dim model → only first 1024 used.
    let mut floats = vec![0.0f32; 1536];
    for (i, v) in floats.iter_mut().enumerate().take(1024) {
        *v = if i % 2 == 0 { 1.0 } else { -1.0 };
    }

    let emb = binarize(&floats, Threshold::Median);
    let words = emb.into_inner();
    for word in &words {
        assert_eq!(*word, 0x5555555555555555);
    }
}

#[test]
fn binarize_empty() {
    assert_eq!(
        binarize(&[], Threshold::Median),
        strata_core::BinaryEmbedding::default()
    );
}
