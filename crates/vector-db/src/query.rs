use strata_core::{BinaryEmbedding, MemoryEntry};

/// Result of a nearest-neighbor query.
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub entry: MemoryEntry,
    pub distance: u32,
}

/// Compute hamming distance between two binary embeddings.
pub fn hamming_distance(a: &BinaryEmbedding, b: &BinaryEmbedding) -> u32 {
    a.as_words()
        .iter()
        .zip(b.as_words().iter())
        .map(|(x, y)| (x ^ y).count_ones())
        .sum()
}

/// Flat-scan the index for the top-k nearest entries by hamming distance.
pub fn top_k(index: &[MemoryEntry], query: &BinaryEmbedding, k: usize) -> Vec<QueryResult> {
    let mut results: Vec<QueryResult> = index
        .iter()
        .map(|entry| QueryResult {
            entry: *entry,
            distance: hamming_distance(&entry.embedding, query),
        })
        .collect();

    results.sort_by_key(|r| r.distance);
    results.truncate(k);
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::{ContentHash, MemoryId};

    fn make_entry(id: u64, words: [u64; 4]) -> MemoryEntry {
        MemoryEntry::new(
            MemoryId::new(id),
            BinaryEmbedding::new(words),
            ContentHash::new([0u8; 32]),
        )
    }

    #[test]
    fn hamming_distance_identical() {
        let a = BinaryEmbedding::new([0, 0, 0, 0]);
        assert_eq!(hamming_distance(&a, &a), 0);
    }

    #[test]
    fn hamming_distance_all_different() {
        let a = BinaryEmbedding::new([0, 0, 0, 0]);
        let b = BinaryEmbedding::new([u64::MAX, u64::MAX, u64::MAX, u64::MAX]);
        assert_eq!(hamming_distance(&a, &b), 256);
    }

    #[test]
    fn hamming_distance_single_bit() {
        let a = BinaryEmbedding::new([0, 0, 0, 0]);
        let b = BinaryEmbedding::new([1, 0, 0, 0]);
        assert_eq!(hamming_distance(&a, &b), 1);
    }

    #[test]
    fn top_k_returns_nearest() {
        let entries = vec![
            make_entry(0, [0xFF, 0, 0, 0]),
            make_entry(1, [0, 0, 0, 0]),
            make_entry(2, [1, 0, 0, 0]),
        ];
        let query = BinaryEmbedding::new([0, 0, 0, 0]);
        let results = top_k(&entries, &query, 2);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].entry.id, MemoryId::new(1));
        assert_eq!(results[0].distance, 0);
        assert_eq!(results[1].entry.id, MemoryId::new(2));
        assert_eq!(results[1].distance, 1);
    }

    #[test]
    fn top_k_larger_than_index() {
        let entries = vec![make_entry(0, [0, 0, 0, 0])];
        let query = BinaryEmbedding::new([0, 0, 0, 0]);
        let results = top_k(&entries, &query, 10);
        assert_eq!(results.len(), 1);
    }
}
