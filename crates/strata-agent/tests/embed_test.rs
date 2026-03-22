//! Tests for the public embedding API.

use strata_agent::embed::{Threshold, binarize};
#[cfg(feature = "local-embed")]
use strata_agent::embed::{Embedder, LocalEmbedder};
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

// --- Local ONNX embedder tests (require model files + local-embed feature) ---

#[cfg(feature = "local-embed")]
fn model_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../models/embed")
}

#[cfg(feature = "local-embed")]
fn has_model() -> bool {
    let dir = model_dir();
    dir.join("model.onnx").exists() && dir.join("tokenizer.json").exists()
}

#[cfg(feature = "local-embed")]
#[tokio::test]
async fn local_embedder_loads_and_produces_output() {
    if !has_model() {
        eprintln!("skipping: model files not found in models/embed/");
        return;
    }

    let embedder = LocalEmbedder::mixedbread(model_dir()).expect("failed to load model");
    let emb = embedder.embed("Hello, world!").await.expect("embed failed");

    // Should produce a non-zero embedding.
    let words = emb.as_words();
    let set_bits: u32 = words.iter().map(|w| w.count_ones()).sum();
    assert!(set_bits > 0, "embedding is all zeros");
    assert!(set_bits < 1024, "embedding is all ones");
}

#[cfg(feature = "local-embed")]
#[tokio::test]
async fn local_embedder_similar_texts_are_closer() {
    if !has_model() {
        eprintln!("skipping: model files not found in models/embed/");
        return;
    }

    let embedder = LocalEmbedder::mixedbread(model_dir()).expect("failed to load model");

    let cat1 = embedder.embed("The cat sat on the mat").await.unwrap();
    let cat2 = embedder.embed("A cat was sitting on a mat").await.unwrap();
    let unrelated = embedder.embed("Quantum computing uses qubits for parallel computation").await.unwrap();

    // Hamming distance: lower = more similar.
    let dist_similar = hamming(&cat1, &cat2);
    let dist_different = hamming(&cat1, &unrelated);

    assert!(
        dist_similar < dist_different,
        "similar texts should have lower hamming distance: similar={dist_similar}, different={dist_different}"
    );
}

#[cfg(feature = "local-embed")]
fn hamming(a: &strata_core::BinaryEmbedding, b: &strata_core::BinaryEmbedding) -> u32 {
    a.as_words()
        .iter()
        .zip(b.as_words().iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum()
}
