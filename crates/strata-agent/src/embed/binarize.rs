//! Float-to-binary embedding conversion.

use strata_core::{BinaryEmbedding, EMBEDDING_WORDS};

/// Maximum number of float dimensions to binarize (one per bit).
const BINARY_DIMS: usize = EMBEDDING_WORDS * 64; // 1024

/// Threshold strategy for binarizing float embeddings.
#[derive(Clone, Copy, Debug, Default)]
pub enum Threshold {
    /// Threshold at the median value. Guarantees ~50% bits set.
    /// Best for models not trained for binary quantization (OpenAI, Cohere).
    #[default]
    Median,
    /// Threshold at zero. Positive → 1, non-positive → 0.
    /// Best for models trained for binary quantization (mixedbread).
    Zero,
}

/// Convert a float embedding to a 1024-bit binary embedding.
///
/// Uses the first `min(len, 1024)` dimensions. Models with fewer dimensions
/// leave upper bits as zero, which is harmless for hamming distance.
pub fn binarize(floats: &[f32], threshold: Threshold) -> BinaryEmbedding {
    let dims = floats.len().min(BINARY_DIMS);
    if dims == 0 {
        return BinaryEmbedding::default();
    }
    let slice = &floats[..dims];

    let cutoff = match threshold {
        Threshold::Zero => 0.0,
        Threshold::Median => compute_median(slice),
    };

    // Pack bits into u64 words.
    let mut words = [0u64; EMBEDDING_WORDS];
    for (i, &val) in slice.iter().enumerate() {
        if val > cutoff {
            words[i / 64] |= 1u64 << (i % 64);
        }
    }

    BinaryEmbedding::new(words)
}

fn compute_median(slice: &[f32]) -> f32 {
    let mut sorted = slice.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 && mid > 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_half_negative_half_positive() {
        let mut floats = vec![0.0f32; 1024];
        for (i, v) in floats.iter_mut().enumerate() {
            *v = if i < 512 { -1.0 } else { 1.0 };
        }

        let emb = binarize(&floats, Threshold::Median);
        let words = emb.into_inner();
        for word in &words[..8] {
            assert_eq!(*word, 0);
        }
        for word in &words[8..] {
            assert_eq!(*word, u64::MAX);
        }
    }

    #[test]
    fn zero_threshold_positive_negative() {
        let mut floats = vec![0.0f32; 1024];
        for (i, v) in floats.iter_mut().enumerate() {
            *v = if i < 512 { -1.0 } else { 1.0 };
        }

        // With zero threshold, same result as median for symmetric data.
        let emb = binarize(&floats, Threshold::Zero);
        let words = emb.into_inner();
        for word in &words[..8] {
            assert_eq!(*word, 0);
        }
        for word in &words[8..] {
            assert_eq!(*word, u64::MAX);
        }
    }

    #[test]
    fn zero_threshold_all_positive() {
        // All values positive → all bits set.
        let floats = vec![1.0f32; 1024];
        let emb = binarize(&floats, Threshold::Zero);
        let words = emb.into_inner();
        for word in &words {
            assert_eq!(*word, u64::MAX);
        }

        // Median threshold → all same value → no bits set.
        let emb2 = binarize(&floats, Threshold::Median);
        let words2 = emb2.into_inner();
        for word in &words2 {
            assert_eq!(*word, 0);
        }
    }

    #[test]
    fn empty_input() {
        assert_eq!(
            binarize(&[], Threshold::Median),
            BinaryEmbedding::default()
        );
    }

    #[test]
    fn shorter_model_zero_pads() {
        let mut floats = vec![0.0f32; 768];
        for (i, v) in floats.iter_mut().enumerate() {
            *v = if i % 2 == 0 { 1.0 } else { -1.0 };
        }

        let emb = binarize(&floats, Threshold::Median);
        let words = emb.into_inner();
        for word in &words[..12] {
            assert_eq!(*word, 0x5555555555555555);
        }
        for word in &words[12..] {
            assert_eq!(*word, 0);
        }
    }

    #[test]
    fn deterministic() {
        let floats: Vec<f32> = (0..1024).map(|i| (i as f32).sin()).collect();
        let a = binarize(&floats, Threshold::Median);
        let b = binarize(&floats, Threshold::Median);
        assert_eq!(a, b);

        let set_bits: u32 = a.as_words().iter().map(|w| w.count_ones()).sum();
        assert!(
            set_bits > 400 && set_bits < 624,
            "set_bits={set_bits}, expected ~512"
        );
    }
}
